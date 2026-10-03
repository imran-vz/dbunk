//! Launch modes never infer endpoint authority from a directory's contents.
//! General profile creation and opening are separate explicit operations.
use dbunk_lib::backend::{Backend, DevelopmentFixtures};
use std::{ffi::OsString, io::Read, path::PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub enum Launch {
    Fixture(PathBuf),
    FixtureWorkspace { profile: PathBuf, manifest: PathBuf },
    Native { profile: PathBuf, create: bool },
}

impl Launch {
    pub fn parse(
        args: impl IntoIterator<Item = OsString>,
        fixture_verification_requested: bool,
    ) -> anyhow::Result<Self> {
        let mut args = args.into_iter();
        let mode = args.next().ok_or_else(|| anyhow::anyhow!(
            "Usage: dbunk-native --profile PATH | --workspace-profile PATH --fixture-manifest FILE | --create-native-profile PATH | --native-profile PATH"
        ))?;
        let profile = PathBuf::from(
            args.next()
                .ok_or_else(|| anyhow::anyhow!("Missing profile path"))?,
        );
        anyhow::ensure!(
            profile.is_absolute(),
            "An absolute profile path is required"
        );
        let launch = if mode == "--profile" {
            Self::Fixture(profile)
        } else if mode == "--workspace-profile" {
            anyhow::ensure!(
                args.next().as_deref() == Some(std::ffi::OsStr::new("--fixture-manifest")),
                "A verified fixture manifest is required"
            );
            let manifest = PathBuf::from(
                args.next()
                    .ok_or_else(|| anyhow::anyhow!("Missing fixture manifest"))?,
            );
            anyhow::ensure!(
                manifest.is_absolute(),
                "Fixture manifest path must be absolute"
            );
            Self::FixtureWorkspace { profile, manifest }
        } else if mode == "--native-profile" || mode == "--create-native-profile" {
            anyhow::ensure!(
                !fixture_verification_requested,
                "Fixture verification cannot run with a general native profile"
            );
            Self::Native {
                profile,
                create: mode == "--create-native-profile",
            }
        } else {
            anyhow::bail!("Unknown native launch mode");
        };
        anyhow::ensure!(
            args.next().is_none(),
            "Unexpected or mixed native launch arguments"
        );
        Ok(launch)
    }

    pub fn workspace(&self) -> bool {
        !matches!(self, Self::Fixture(_))
    }

    /// Constructors independently verify canonical paths, mode-specific marker,
    /// database identity and exclusive ownership before credential access.
    pub async fn open(&self) -> anyhow::Result<Backend> {
        let result = match self {
            Self::Fixture(profile) => Backend::open_fixture(profile).await,
            Self::FixtureWorkspace { profile, manifest } => {
                let mut encoded = Vec::new();
                std::fs::File::open(manifest)?
                    .take(4097)
                    .read_to_end(&mut encoded)?;
                anyhow::ensure!(encoded.len() <= 4096, "Fixture manifest is too large");
                let fixtures = DevelopmentFixtures::from_json(std::str::from_utf8(&encoded)?)
                    .map_err(anyhow::Error::msg)?;
                if profile.exists() {
                    Backend::open_development(profile, &fixtures).await
                } else {
                    Backend::create_development(profile, fixtures).await
                }
            }
            Self::Native {
                profile,
                create: true,
            } => Backend::create_native_profile(profile).await,
            Self::Native {
                profile,
                create: false,
            } => Backend::open_native_profile(profile).await,
        };
        result.map_err(anyhow::Error::msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str], verification: bool) -> anyhow::Result<Launch> {
        Launch::parse(args.iter().map(OsString::from), verification)
    }

    #[test]
    fn modes_have_distinct_create_and_endpoint_authority() {
        assert_eq!(
            parse(&["--profile", "/owned/profile"], false).unwrap(),
            Launch::Fixture("/owned/profile".into())
        );
        assert_eq!(
            parse(
                &[
                    "--workspace-profile",
                    "/owned/profile",
                    "--fixture-manifest",
                    "/owned/manifest"
                ],
                false
            )
            .unwrap(),
            Launch::FixtureWorkspace {
                profile: "/owned/profile".into(),
                manifest: "/owned/manifest".into()
            }
        );
        for (flag, create) in [
            ("--native-profile", false),
            ("--create-native-profile", true),
        ] {
            let launch = parse(&[flag, "/owned/profile"], false).unwrap();
            assert_eq!(
                launch,
                Launch::Native {
                    profile: "/owned/profile".into(),
                    create
                }
            );
            assert!(launch.workspace());
            assert!(parse(&[flag, "/owned/profile"], true).is_err());
        }
        assert!(
            !parse(&["--profile", "/owned/profile"], true)
                .unwrap()
                .workspace()
        );
    }

    #[test]
    fn missing_relative_mixed_and_extra_arguments_never_select_a_fallback() {
        for args in [
            vec![],
            vec!["--native-profile"],
            vec!["--native-profile", "relative"],
            vec!["--workspace-profile", "/owned/profile"],
            vec![
                "--workspace-profile",
                "/owned/profile",
                "--fixture-manifest",
                "relative",
            ],
            vec![
                "--native-profile",
                "/owned/profile",
                "--fixture-manifest",
                "/owned/manifest",
            ],
            vec![
                "--profile",
                "/owned/profile",
                "--native-profile",
                "/another/profile",
            ],
            vec!["--create-native-profile", "/owned/profile", "extra"],
            vec!["--unknown", "/owned/profile"],
        ] {
            assert!(parse(&args, false).is_err(), "accepted {args:?}");
        }
    }
}
