# Diagnosis integration review

Independent read-only review covered the new backend facade, shared probe
preparation, form state/revision logic, Host worker and workspace integration.
No concrete correctness findings were reported. The reviewer checked profile
authority before admission, stored-credential boundary checks before hydration,
development/credential guards, queued and late-delivery cancellation, input-edit
invalidation, Drop cancellation, retained joins and shared payload allowances.
No builds, fixtures or UI were used by that review.

Root's protocol review found that replacing the former native Test action could
stop validating configured session options. The runner now applies the same
ordered PostgreSQL option statements in its disposable database-stage session.
A synthetic missing-role failure must report Database failure after successful
authentication, close the socket and never return Reachable. The source manifest
distinguishes this correction from the initial checks. Final checks must cover
the corrected runner; early passing logs alone do not establish that coverage.

The report's client-certificate false value is rendered as “not observed”, since
absence of a server observation cannot prove that no certificate was presented.
The accepted asynchronous-deadline/platform-I/O limits are in the progress note.
