# Native XLSX dependency-edge review, 2026-10-03

Plans 027/029 require review before dependency changes. The bounded native
reader directly declares the existing pinned zip 2.4.2 and quick-xml 0.31.0
packages. Both locks already contain these exact versions and checksums.
Calamine already enables zip with defaults disabled plus deflate and quick-xml
with encoding; the new declarations request precisely those features. ZIP
compression/encryption defaults remain disabled. Both packages are MIT; exact
installed package license texts were added to the native packaged notices.

Independent pinned-source inspection established the parser/allocation behavior
and feature/license compatibility. Root then compared complete before/after
Cargo metadata for the backend Apple Silicon isolated target and native host.
[Graph comparison](./dependency-graph-review.json) proves identical package IDs,
versions, sources, licenses, resolved-node IDs and feature sets. Only dbunk's
own dependency edges changed. No dependency upgrade or new package was made.

The first offline backend metadata request tried unrelated target packages and
refused an uncached Linux dbus download. Restricting metadata to the authorized
Apple Silicon target succeeded offline. This was not a build failure and no
package was downloaded to bypass the check. Full locked checks, dependency proof
and packaged-notice verification remain required after implementation.

The reader will not activate the legacy all-sheets dense parser. Pinned ZIP
retries earlier EOCD candidates, so preflight must cover all candidates before
library allocation. Pinned Quick XML copies opening names internally; bounded
entry buffers plus a structural pass must precede shared-string/row retention.
These findings are implementation requirements, not acceptance claims.
