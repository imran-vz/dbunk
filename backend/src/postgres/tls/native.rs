//! Same verifier policy as other drivers, with bounded opened-file PEM reads.
//! Local filesystem and platform-root operations are synchronous: the async
//! probe deadline is not a hard wall-clock bound across those operations.
use super::*;
use std::io::Read;
const MAX_PEM_BYTES: usize = 1024 * 1024;
fn read_regular(path: &Path) -> Result<Vec<u8>, TlsMaterialError> {
    let error = || TlsMaterialError::Unreadable {
        path: display_path(path),
        detail: "Native TLS material must be a readable regular file of at most 1 MiB".into(),
    };
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|_| error())?;
    let metadata = file.metadata().map_err(|_| error())?;
    if !metadata.is_file() || metadata.len() > MAX_PEM_BYTES as u64 {
        return Err(error());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take((MAX_PEM_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| error())?;
    if bytes.len() > MAX_PEM_BYTES {
        return Err(error());
    }
    Ok(bytes)
}
pub(crate) fn client_config(
    tls: &ResolvedTls,
) -> Result<Option<Arc<rustls::ClientConfig>>, TlsMaterialError> {
    if tls.mode == PgTlsMode::Disable {
        return Ok(None);
    }
    let auth = match (&tls.client_cert_path, &tls.client_key_path) {
        (None, None) => None,
        (Some(cert), Some(key)) => Some((
            parse_certs(cert, &read_regular(cert)?)?,
            parse_private_key(key, &read_regular(key)?)?,
        )),
        (Some(path), None) | (None, Some(path)) => {
            return Err(TlsMaterialError::ClientPairIncomplete {
                present: if tls.client_cert_path.is_some() {
                    "certificate"
                } else {
                    "key"
                },
                path: display_path(path),
            })
        }
    };
    let roots = if tls.mode.verifies_chain() {
        let mut roots = RootCertStore::clone(&native_roots().store);
        if let Some(path) = &tls.root_cert_path {
            for certificate in parse_certs(path, &read_regular(path)?)? {
                roots
                    .add(certificate)
                    .map_err(|error| TlsMaterialError::Malformed {
                        path: display_path(path),
                        detail: error.to_string(),
                    })?;
            }
        }
        Some(Arc::new(roots))
    } else {
        None
    };
    config_from_parts(tls, auth, roots)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_opened_material_refuses_oversize_directory_and_symlink() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("material");
        std::fs::write(&path, b"bounded exact bytes").unwrap();
        assert_eq!(read_regular(&path).unwrap(), b"bounded exact bytes");
        std::fs::File::create(&path)
            .unwrap()
            .set_len((MAX_PEM_BYTES + 1) as u64)
            .unwrap();
        assert!(read_regular(&path).is_err());
        assert!(read_regular(directory.path()).is_err());
        #[cfg(unix)]
        {
            let link = directory.path().join("link");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(read_regular(&link).is_err());
        }
    }
    #[test]
    fn native_and_legacy_preserve_mode_and_incomplete_client_pair_policy() {
        let mut tls = ResolvedTls::plain("localhost");
        tls.client_cert_path = Some(PathBuf::from("missing-cert"));
        assert!(client_config(&tls).unwrap().is_none());
        assert!(super::super::client_config(&tls).unwrap().is_none());
        tls.mode = PgTlsMode::Require;
        assert!(matches!(
            client_config(&tls),
            Err(TlsMaterialError::ClientPairIncomplete { .. })
        ));
        assert!(matches!(
            super::super::client_config(&tls),
            Err(TlsMaterialError::ClientPairIncomplete { .. })
        ));
        tls.client_cert_path = None;
        assert!(client_config(&tls).unwrap().is_some());
        assert!(super::super::client_config(&tls).unwrap().is_some());
    }
}
