//! The app's asset source: Zed's bundled assets plus dbunk's own engine
//! logos, which are embedded from the repository's `logos/` directory.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

const LOGOS: [(&str, &[u8]); 4] = [
    (
        "logos/postgres.svg",
        include_bytes!("../../../logos/postgres.svg"),
    ),
    (
        "logos/mysql.svg",
        include_bytes!("../../../logos/mysql.svg"),
    ),
    (
        "logos/clickhouse.svg",
        include_bytes!("../../../logos/clickhouse.svg"),
    ),
    (
        "logos/redis.svg",
        include_bytes!("../../../logos/redis.svg"),
    ),
];

pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match LOGOS.iter().find(|(logo, _)| *logo == path) {
            Some((_, data)) => Ok(Some(Cow::Borrowed(data))),
            None => assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = assets::Assets.list(path)?;
        paths.extend(
            LOGOS
                .iter()
                .filter(|(logo, _)| logo.starts_with(path))
                .map(|(logo, _)| SharedString::from(*logo)),
        );
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use resvg::usvg;

    #[test]
    fn engine_logos_load_and_parse_as_svg() {
        for engine in ["PostgreSQL", "MySQL", "ClickHouse", "Redis"] {
            let path = crate::style::engine_logo(engine).expect("engine has a logo");
            let data = AppAssets.load(path).unwrap().expect("logo is embedded");
            usvg::Tree::from_data(&data, &usvg::Options::default())
                .unwrap_or_else(|error| panic!("{path} does not parse: {error}"));
        }
        assert_eq!(crate::style::engine_logo("SQLite"), None);
    }

    #[test]
    fn other_paths_fall_through_to_zed_assets() {
        assert!(AppAssets.load("icons/check.svg").unwrap().is_some());
        assert!(
            AppAssets
                .list("logos/")
                .unwrap()
                .contains(&SharedString::from("logos/redis.svg"))
        );
    }
}
