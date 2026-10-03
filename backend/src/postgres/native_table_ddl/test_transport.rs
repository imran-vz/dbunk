//! Test-only owned socket: deterministic namespace barriers and acknowledgement loss.
use super::*;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum HookPoint {
    Captured(u8),
    /// PostgreSQL has acknowledged COMMIT, but the runner has not received it.
    Committed,
}
pub(crate) type Hook = Box<dyn FnMut(HookPoint) -> BoxFuture<'static, Result<(), Failure>> + Send>;
struct Hooked {
    socket: Socket,
    captures: u8,
    hook: Hook,
}
impl Transport for Hooked {
    fn execute<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), Failure>> {
        Box::pin(async move {
            self.socket.execute(sql).await?;
            if sql == "COMMIT" {
                (self.hook)(HookPoint::Committed).await?;
            }
            Ok(())
        })
    }
    fn capture<'a>(
        &'a mut self,
        request: &'a TableDdlRequest,
    ) -> BoxFuture<'a, Result<TableDdlDescription, Failure>> {
        Box::pin(async move {
            let result = self.socket.capture(request).await?;
            self.captures += 1;
            // Called after SQL snapshot completes, before the runner acts on it.
            (self.hook)(HookPoint::Captured(self.captures)).await?;
            Ok(result)
        })
    }
    fn locked(&mut self, identity: TableIdentity) -> BoxFuture<'_, Result<bool, Failure>> {
        self.socket.locked(identity)
    }
    fn cleanup(self, cancel: bool, deadline: Instant) -> BoxFuture<'static, ()> {
        self.socket.cleanup(cancel, deadline)
    }
}
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute(
    spec: ResolvedPostgresConnectSpec,
    drivers: DriverJoins,
    permit: WritePermit,
    cancellation: watch::Receiver<u64>,
    target: TableDdlDescription,
    intent: TableDdlIntent,
    preview: TableDdlPreview,
    hook: Hook,
) -> Outcome {
    let deadline = Instant::now() + Duration::from_secs(30);
    match tokio::time::timeout_at(
        deadline,
        dedicated::connect_tracked(&spec, NoticeSink::Ignore, Some(&drivers)),
    )
    .await
    {
        Ok(Ok(connection)) => {
            if let Ok(row) = connection
                .client
                .query_one("SELECT pg_catalog.pg_backend_pid()", &[])
                .await
            {
                println!(
                    "owned table_ddl race/cancel backend_pid={}",
                    row.get::<_, i32>(0)
                );
            }
            run(
                Hooked {
                    socket: Socket {
                        connection,
                        drivers,
                    },
                    captures: 0,
                    hook,
                },
                &permit,
                cancellation,
                &target,
                &intent,
                &preview,
                deadline,
            )
            .await
        }
        _ => {
            join(&drivers, deadline).await;
            Outcome::NotDispatched {
                reason: Failure::Connection,
            }
        }
    }
}
