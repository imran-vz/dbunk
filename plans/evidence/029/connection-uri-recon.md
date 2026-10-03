# PostgreSQL connection URI reconnaissance

2026-10-03. Source inspection only, against Tauri baseline `102568b` and the current general-profile native source. No source changes, tests, SQL, network, profiles, credentials or UI were accessed. This is a proposed bounded implementation contract, not acceptance evidence. It covers the URI portion of Plan029 T14, not SSH, managed connections or full staged diagnosis.

## Baseline source authority

Read with `git show 102568b:<path>`:

- `src/lib/connection-uri.ts`: `parseConnectionUri`, `buildConnectionUri`, `resolvedPgTlsMode`.
- `src/lib/connection-uri.test.ts`: encoding, password import, IPv6, defaults, ignored parameters, unsupported schemes and five-mode TLS corpus.
- `src/components/connection-actions.tsx`: `Copy URI`, password-omission notice and clipboard-failure notice.
- `src/components/connection-form/uri-import-field.tsx` and `.test.tsx`: prefill behavior, incomplete host, atomic parse failure, ignored-parameter disclosure and engine handling.
- `src/components/connection-form.tsx`: URI import appears only in the new-connection form.
- `src/components/connection-form/{use-connection-form,form-utils}.ts`: form defaults, password handling, explicit Test/Save and TLS field persistence.
- `src/lib/store/types.ts`: `PG_TLS_MODES`; `src/lib/engine-policy.ts`: PostgreSQL port 5432 and form validation.

## Exact PostgreSQL behavior

| Concern | Baseline behavior |
| --- | --- |
| Accepted input | Trim outer whitespace. Require `scheme://`. Accept `postgres` and `postgresql`, case-insensitively. The shared baseline also accepts MySQL/Redis, which are outside this PostgreSQL slice. No keyword DSN or arbitrary libpq syntax parser is present. |
| Parse errors | Empty: “Paste a connection URI first.” Missing URI scheme, unsupported scheme and URL parse failure have distinct reasons plus accepted schemes. Errors are returned before any form fields change. Unsupported-scheme copy interpolates the submitted scheme; a native error need not echo input. |
| Parser | JavaScript WHATWG `URL`, not a database connection or libpq parser. No endpoint validation, DNS, certificate read, secret lookup or socket is part of parsing. |
| Host | `url.hostname`, with enclosing IPv6 brackets removed. Missing host can parse, but the import field applies nothing and says to add a host. |
| Port | Explicit URL port parsed as decimal; absent means 5432. URI parsing itself has no explicit nonzero check. Form validation is separate. |
| User/password | `decodeURIComponent` of URL username/password. Password is returned only when `url.password` is nonempty; absent or explicitly empty password does not overwrite an existing typed password. User is always assigned, even when empty. `+` in userinfo is a literal plus, not a form-urlencoded space. |
| Database | Remove one leading slash, take the first raw path segment, then percent-decode. Thus encoded `%2F` belongs to one database name, while `/first/second` silently keeps only `first`. Empty/missing path assigns an empty database. |
| Percent decoding | Valid UTF-8 percent escapes decode. If `decodeURIComponent` throws, baseline returns that entire component unchanged. This can leave malformed escapes or invalid UTF-8 escape sequences as literal text. |
| TLS | Only exact lowercase `disable`, `prefer`, `require`, `verify-ca`, `verify-full` are recognized, matching `PG_TLS_MODES`. `allow`, differently cased values and other strings are not applied and remain ignored parameters. |
| Query parameters | `URLSearchParams.get("sslmode")` takes the first occurrence. All query keys are deduplicated in encounter order. A recognized sslmode removes that key from ignored parameters; everything else is ignored, including `host`, `port`, `user`, `password`, `connect_timeout`, `options`, `sslrootcert`, `sslcert`, `sslkey`. Certificate paths are deliberately not trusted from pasted URIs. |
| Duplicate sslmode | First occurrence wins; if valid, subsequent occurrences are not separately disclosed. This can conceal a conflicting second value. |
| Fragment | Not inspected or disclosed by the parser. PostgreSQL parser warnings are otherwise empty; the implemented `warnings` cases concern Redis database numbers. |
| New-form application | Host, port, user and database always replace the corresponding fields once a host exists. Password and TLS mode replace their fields only when returned. Name, environment, policy, organization, certificate paths and driver fields are not reset. Same engine does not trigger an engine reset. Import never saves, tests or connects. |
| Empty typing | An empty import input clears the notice only. `postgres://` shows “Add a host to apply the URI.” and leaves existing fields alone. |

The import component's comment says “only fields the URI carries”; the actual code unconditionally applies user and database, including empty strings. Preserve the actual behavior or explicitly record a deliberate difference.

### Secret-free export

The builder's input type has no password field and no include-secret option. PostgreSQL always emits `postgres://`, not `postgresql://`. Nonempty user becomes an encoded userinfo component plus `@`; empty user omits userinfo. Empty host falls back to `localhost`; port zero falls back to 5432. Bare colon-containing hosts are bracketed; already bracketed hosts are left alone. A nonempty database is one encoded path segment; an empty database omits the slash/path.

User/database encoding is JavaScript `encodeURIComponent`: ASCII letters, digits and `- _ . ! ~ * ' ( )` remain; other UTF-8 bytes use percent escapes. Despite the source comment's “RFC 3986” wording, those literal punctuation characters are the exact implemented behavior. Do not substitute query-form encoding (`+` for space), decode database slashes before splitting, or percent-encode an already encoded value twice.

TLS resolution is `tlsOptions.mode`, otherwise `ssl === false ? disable : prefer`. Export appends `?sslmode=<mode>` for every supported mode except `prefer`, which is omitted. Baseline tests establish legacy false → disable and absent/true → prefer.

Copy URI uses already available connection metadata, writes only the generated URI to the clipboard, and reports that the password was omitted. Clipboard rejection has a separate failure notice. It does not retrieve a credential. SQLite and ClickHouse hide this action because the shared builder refuses them; native PostgreSQL can keep unsupported records inactive rather than enabling other engines.

### What this URI does not reproduce

This baseline URI is an endpoint/user/database/TLS-mode summary. It does not export passwords; CA/client-certificate/key paths; alternate verification server name; SSH bastion, jump, proxy, host-key or verification configuration; driver timeouts, keepalive, role/search_path; read-only/safe-mode/environment policy; organization, managed lifecycle identity or saved connection identity.

The baseline copy action discloses password omission only, even for records with those other settings. Import names ignored query keys and tells the user to set matching options below. No baseline-specific support was found for Unix socket authorities, multiple libpq hosts/ports, service files or keyword/value DSNs. Do not claim their support merely because the URL parser accepts some characters. A native export notice can accurately add “TLS files and connection options are not included” when relevant, without changing transport behavior.

## Current native integration seams

- `apps/native/src/forms.rs`, `Form::connection`, `field`, `connection_input`, `submit`: existing single-line editors, masked password field, TLS selector, driver fields, explicit Test/Save and error message. There is no URI import editor/action. `Kind::Connection { id: None }` is the baseline-equivalent new-form scope; an edit form starts with a blank password and uses existing saved-secret semantics on Test/Save.
- `forms.rs::connection_defaults`: general mode uses 127.0.0.1:5432, database/user `postgres`; owned-fixture mode keeps 15432/dbunk_demo/dbunk. Both currently use TLS `disable`. Baseline new-form TLS is `prefer`. Since URI import without sslmode preserves the current mode, blindly adding the baseline patch algorithm leaves an important default difference. Choose explicitly: retain visible native mode and disclose it, or align general new-form TLS with baseline `prefer` as a separately stated default change. Do not silently reset an explicitly chosen mode on every URI edit, and do not change fixture TLS admission.
- `apps/native/src/workspace.rs`, `Operation`, `activate`, connection-row controls: existing Edit, “Copy” (currently duplicate), favorite and Delete. Add an unambiguous `Copy URI` action beside the existing duplicate action. Read `DevelopmentConnection.postgres`; no credential service call is necessary. Only expose the action for supported PostgreSQL metadata. Use existing status and clipboard facilities.
- `src-tauri/src/backend/development/connections.rs`: `DevelopmentPostgresConnection` is secret-free and contains resolved TLS/driver metadata; passwords are separate method arguments. Save validates bounded fields and remains storage-only. Test and Connect stay explicit. `summary` and native admission preserve unsupported records while preventing activation.
- `backend/native_profile.rs` and `development::Authority`: general-profile endpoint capability and exact fixture capability remain authoritative at Save/Test/Connect. A URI parser must never select or widen a capability. Parsing a non-fixture URI can prefill a fixture form, but the existing facade must still refuse its use outside the manifest.

The new URI field may contain a password. Treat its editor as secret-bearing, with masked/accessible-secret semantics, finite retained input/history and no raw-URI diagnostics. Apply only after a complete bounded parse; do not overwrite fields while they hold marked IME composition. Clearing the URI after application must also consider editor undo history. The current `field` helper masks secrets but is not itself an input/history budget guard; reuse the existing bounded editor patterns rather than assuming a character limit on submission limits retention.

## Reusable pure facade, no new package required

The backend already directly depends on `reqwest = 0.12` (`src-tauri/Cargo.toml:70`). Its installed `reqwest-0.12.28/src/lib.rs:280` publicly reexports `url::Url`; current code already uses `reqwest::Url` in `src-tauri/src/clickhouse.rs` and `src-tauri/src/tunnel/endpoint.rs`. Calling `reqwest::Url::parse` is pure and does not construct a client or open a socket.

The backend lock pins `url 2.5.8`; the native lock pins `url 2.5.7`. Both already pin `form_urlencoded 1.2.2` and `percent-encoding 2.3.2`. Neither application manifest declares `url` directly. Use the available `reqwest::Url` reexport in the backend module to preserve both locks. Do not assume a transitive crate is directly importable or promote an exact version that updates the other workspace's graph. Native UI calls the backend's pure function and needs no URL dependency.

Suggested new `backend::connection_uri` module with no backend handle or async work:

```rust
pub fn parse_postgres_uri(input: &str) -> Result<ParsedPostgresUri, UriError>;
pub fn build_postgres_uri(input: &DevelopmentPostgresConnection)
    -> Result<ExportedPostgresUri, UriError>;
```

`ParsedPostgresUri` carries bounded `host`, `port`, `user`, `database`, optional password, optional `DevelopmentTlsMode`, ordered unique ignored parameter names, and small typed warnings. It must have redacted Debug and no blanket Serialize/Display exposing the secret or source URI. No raw source URI needs to be retained in this DTO. Missing host should be a distinct incomplete-input outcome/error so the UI can preserve its existing fields. An absent/empty password remains `None` for baseline patch semantics. Errors should be stable typed reasons with secret-free Display, never a formatted URL.

`ExportedPostgresUri` carries the secret-free string and typed omission flags for nonrepresented TLS/driver fields. The builder receives no password argument. Prefer explicit UTF-8 component escaping with the baseline allowlist; URL setters have their own escaping rules and should not be presumed byte-for-byte equivalent. Validate the host as a single authority rather than reproducing the baseline builder's unchecked colon-bracketing for arbitrary malformed stored host strings. Refuse unsupported host forms without inventing or redirecting an endpoint.

Suggested bounds, to finalize at implementation: 16 KiB input before URL parsing, host/user/database at existing 256-byte form limits after decoding, password at 4 KiB, at most 32 query pairs/128 bytes per decoded key, bounded warnings and a 4 KiB export cap checked before escaped output allocation. A bounded decoder can preserve exact UTF-8 or refuse invalid sequences; do not use `query_pairs`' lossy decoding for username/password/database. Query values use form-urlencoded `+` semantics, unlike userinfo/path. Never collect an unbounded query iterator or include ignored query values in notices.

Proposed deliberate correctness differences to record, rather than calling them exact baseline parity:

- Refuse malformed percent escapes, invalid decoded UTF-8, NUL/control characters and invalid/zero ports rather than retaining ambiguous components or deferring every error to Save.
- Refuse extra path segments, fragments and conflicting duplicate sslmode, or provide a bounded explicit warning before applying. Baseline silently drops these. Prefer refusal for ambiguity; identical duplicate sslmode can be accepted only with a defined, tested rule.
- Explicitly refuse unsupported Unix socket/multi-host/keyword DSN forms instead of treating them as an ordinary DNS host. Do not normalize a user-selected endpoint into a different one.
- Keep unknown query options, including certificate paths, unapplied and visible. URI import is not an opportunity to add trusted file-path or SSH configuration.

These are recommendations, not implemented behavior. Source inspection cannot establish every WHATWG parser edge case identically across the two pinned Rust URL versions; test the actual pure facade in both workspace graphs during implementation.

## Focused test corpus and ownership

| Corpus | Required observable result |
| --- | --- |
| `postgres://app_user@db.internal:5433/orders`, `postgresql://admin:s%40cret@10.0.0.1/app` | Exact fields; default 5432 for omitted port; password imported only, never re-exported or present in Debug/errors. |
| Username/database containing spaces, `+`, `%`, `@`, `:`, `/`, `?`, `#`, `!'()*`, non-ASCII and emoji | Exact UTF-8 component round-trip; `%2F` remains part of one database name; query `+` handling does not leak into userinfo. |
| Bare/bracketed IPv6, hostname, IPv4, host delimiters and unsupported socket/multi-host shapes | Correct brackets/bare host round-trip; malformed/unsupported authority refuses without partial application. |
| All five TLS modes, absent mode, legacy resolved modes, invalid `allow`/uppercase/value, duplicate keys | Exact emission and omission; invalid values disclosed; duplicate policy tested; native form default difference explicit. |
| `sslrootcert`, `sslcert`, `sslkey`, `host`, `password`, `connect_timeout`, `options`, repeated unknown keys | No file/driver/secret side effects; ordered unique ignored names only, no ignored values in messages. |
| Empty input, bare scheme, no host, unsupported engine/scheme, malformed percent/UTF-8, NUL, bad/zero/oversize port, fragments, extra path | Stable secret-free error/incomplete outcome; all preexisting form fields preserved on refusal. |
| No password, explicit empty password, imported encoded password, URI reapplication | Absent/empty preserves current password; imported password updates masked field only; no save/connect and no secret-bearing history retained beyond the chosen bound. |
| Input, decoded field, query count/key and output limits at/over boundary | Atomic refusal, no clipped endpoint or partially applied form, no unbounded percent-expansion allocation. |
| Copy supported metadata with TLS files/driver options; unsupported connection | Secret-free clipboard payload plus omission notice; unsupported action inactive; clipboard failure truthful. |
| Fixture and general forms, IME marked text, Tab/Enter/Escape, accessibility | Existing scope and focus paths retained; no action during composition; ordinary import never connects. Real UI acceptance remains separate from pure tests. |

Bounded implementation ownership: one backend owner for new `backend/connection_uri.rs` and child pure tests plus its module export, with no manifest changes. Root/native owner for existing form and workspace action integration and editor retention/focus tests. No profile, connection lifecycle, TLS transport or SSH changes are required for this slice. Existing layouts are already approved; this proposal adds the baseline action/field within those surfaces and requests no new layout approval.
