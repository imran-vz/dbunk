use super::JobContext;
use crate::postgres::{
    connect_spec::ResolvedPostgresConnectSpec,
    schema_compare::{capture::capture_resolved, diff, protocol::*},
};
use crate::{credentials, storage, tunnel, StoredConnection};
use std::{sync::Arc, time::Instant};

struct ResolvedEndpoint {
    spec: ResolvedPostgresConnectSpec,
    // Comparison routes have job-scoped forward ownership. Keeping this guard
    // beside the resolved spec holds the listener through capture and removes
    // only this job's route when setup, capture, or comparison stops.
    _route: tunnel::EphemeralRoute,
}

pub(super) async fn run(
    ctx: JobContext,
    pool: sqlx::SqlitePool,
) -> Result<diff::Comparison, CompareError> {
    let source = resolve(&ctx, &pool, &ctx.request.source.connection_id, Side::Source).await?;
    let same = ctx.request.source.connection_id == ctx.request.target.connection_id;
    let target = if same {
        None
    } else {
        Some(resolve(&ctx, &pool, &ctx.request.target.connection_id, Side::Target).await?)
    };
    let (source, target) = if let Some(target) = target {
        ctx.progress(StatusState::ReadingSource, 0, 0);
        let mut source = capture_resolved(
            &source.spec,
            &[(Side::Source, &ctx.request.source)],
            &ctx.control,
            &ctx.budget,
        )
        .await?;
        let source = source.pop().ok_or(CompareError::Unavailable)?;
        ctx.progress(StatusState::ReadingTarget, source.inventory.len() as u32, 0);
        let mut target = capture_resolved(
            &target.spec,
            &[(Side::Target, &ctx.request.target)],
            &ctx.control,
            &ctx.budget,
        )
        .await?;
        (source, target.pop().ok_or(CompareError::Unavailable)?)
    } else {
        ctx.progress(StatusState::ReadingBoth, 0, 0);
        let mut endpoints = capture_resolved(
            &source.spec,
            &[
                (Side::Source, &ctx.request.source),
                (Side::Target, &ctx.request.target),
            ],
            &ctx.control,
            &ctx.budget,
        )
        .await?;
        let target = endpoints.pop().ok_or(CompareError::Unavailable)?;
        (endpoints.pop().ok_or(CompareError::Unavailable)?, target)
    };
    ctx.progress(
        StatusState::Comparing,
        source.inventory.len() as u32,
        target.inventory.len() as u32,
    );
    let result = diff::compare(
        ctx.identity.clone(),
        source,
        target,
        &ctx.control,
        Instant::now(),
    )?;
    ctx.control.check()?;
    // Activity uses only local storage and the original job deadline.
    for id in [
        &ctx.request.source.connection_id,
        &ctx.request.target.connection_id,
    ] {
        let _ = ctx
            .control
            .wait(storage::touch_connection_activity(&pool, id))
            .await;
    }
    ctx.control.check()?;
    Ok(result)
}

async fn resolve(
    ctx: &JobContext,
    pool: &sqlx::SqlitePool,
    id: &str,
    side: Side,
) -> Result<ResolvedEndpoint, CompareError> {
    let mut connection = ctx
        .control
        .wait(storage::read_connection_by_id(pool, id))
        .await?
        .map_err(|_| CompareError::Unavailable)?
        .ok_or(CompareError::Unavailable)?;
    if !matches!(connection, StoredConnection::PostgreSQL(_)) {
        return Err(CompareError::UnsupportedEngine { side });
    }
    let mode = ctx
        .control
        .wait(credentials::credential_mode(pool))
        .await?
        .map_err(|_| CompareError::Unavailable)?
        .ok_or(CompareError::Unavailable)?;
    ctx.control
        .wait(credentials::hydrate(pool, mode, &mut connection))
        .await?
        .map_err(|_| CompareError::Unavailable)?;
    let StoredConnection::PostgreSQL(postgres) = &mut connection else {
        unreachable!("engine checked before credential hydration")
    };
    // A comparison owns a distinct short-lived listener. A configured fixed
    // port may already belong to the ordinary cached route, so let the OS pick
    // an isolated local port while retaining the stored bind host.
    postgres.ssh_tunnel.local_port = None;
    let control = ctx.control.clone();
    // Unlike a dropped timeout future, this resolver keeps its SSH setup worker
    // joined. Cancellation is checked between hops and before cache publication.
    let route = tunnel::EphemeralRoute::new("schema-compare");
    let resolved = tunnel::resolve_connection_checked(
        pool,
        mode,
        route.key(),
        &connection,
        Arc::new(move || {
            control
                .check()
                .map_err(|_| "Comparison setup stopped".into())
        }),
    )
    .await;
    ctx.control.check()?;
    let spec = ResolvedPostgresConnectSpec::from_connection(
        &resolved.map_err(|_| CompareError::Unavailable)?,
    )
    .map_err(|_| CompareError::UnsupportedEngine { side })?;
    Ok(ResolvedEndpoint {
        spec,
        _route: route,
    })
}
