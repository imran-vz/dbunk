use super::super::*;

pub const MAX_DIAGNOSIS_BYTES: usize = 64 * 1024;
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NativeDiagnosis {
    /// Fixed order: tunnel, dns, tcp, tls, authentication, database.
    pub stages: Vec<NativeDiagnosisStage>,
    pub outcome: NativeDiagnosisOutcome,
    pub warnings: Vec<NativeDiagnosisWarning>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NativeDiagnosisStage {
    pub stage: NativeStageKind,
    pub result: NativeStageResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NativeStageKind {
    Tunnel,
    Dns,
    Tcp,
    Tls,
    Authentication,
    Database,
}

const NATIVE_STAGE_ORDER: [NativeStageKind; 6] = [
    NativeStageKind::Tunnel,
    NativeStageKind::Dns,
    NativeStageKind::Tcp,
    NativeStageKind::Tls,
    NativeStageKind::Authentication,
    NativeStageKind::Database,
];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum NativeStageResult {
    Passed {
        elapsed_ms: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<NativeStageDetail>,
    },
    Failed {
        elapsed_ms: u64,
        kind: NativeFailureKind,
        message: String,
    },
    Skipped {
        reason: NativeSkipReason,
    },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum NativeStageDetail {
    Tunnel {
        local_endpoint: String,
    },
    Dns {
        addresses: Vec<String>,
    },
    Tls {
        /// From `pg_stat_ssl` when the session got that far, else from the
        /// handshake outcome. The only honest source for `prefer`.
        encrypted: bool,
        protocol: Option<String>,
        cipher: Option<String>,
        certificate_verified: bool,
        hostname_verified: bool,
        /// True only after a successful `pg_stat_ssl` observation. False can
        /// also mean startup did not reach that optional observation.
        client_certificate_presented: bool,
        pool_hostname_verification_ca_only: bool,
    },
    Database {
        server_version: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NativeFailureKind {
    TunnelFailed,
    DnsUnresolvable,
    ConnectionRefused,
    TimedOut,
    Unreachable,
    ServerRefusedTls,
    CertificateUntrusted,
    HostnameMismatch,
    ClientCertificateRejected,
    InvalidLocalMaterial,
    HandshakeFailed,
    AuthenticationFailed,
    DatabaseMissing,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NativeSkipReason {
    NoTunnel,
    TlsDisabled,
    BlockedByEarlierFailure,
    NotApplicable,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum NativeDiagnosisOutcome {
    Reachable { latency_ms: u64 },
    Failed { stage: NativeStageKind },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NativeDiagnosisWarning {
    NotEncrypted,
    PoolHostnameVerificationCaOnly,
    ProductionWithoutVerification,
    TransactionPooler,
    SessionOptionsNotApplied,
}

macro_rules! same_enum {
    ($value:expr, $source:ident, $target:ident, $($variant:ident),+ $(,)?) => {
        match $value {
            $($source::$variant => $target::$variant,)+
        }
    };
}
impl NativeDiagnosis {
    pub(super) fn from_legacy(report: ConnectionDiagnosis) -> Self {
        Self {
            outcome: match report.outcome {
                DiagnosisOutcome::Reachable { latency_ms } => {
                    NativeDiagnosisOutcome::Reachable { latency_ms }
                }
                DiagnosisOutcome::Failed { stage } => NativeDiagnosisOutcome::Failed {
                    stage: stage_kind(stage),
                },
            },
            warnings: report
                .warnings
                .into_iter()
                .map(|warning| {
                    same_enum!(
                        warning,
                        DiagnosisWarning,
                        NativeDiagnosisWarning,
                        NotEncrypted,
                        PoolHostnameVerificationCaOnly,
                        ProductionWithoutVerification,
                        TransactionPooler,
                        SessionOptionsNotApplied
                    )
                })
                .collect(),
            stages: report
                .stages
                .into_iter()
                .map(|stage| NativeDiagnosisStage {
                    stage: stage_kind(stage.stage),
                    result: match stage.result {
                        StageResult::Skipped { reason } => NativeStageResult::Skipped {
                            reason: same_enum!(
                                reason,
                                SkipReason,
                                NativeSkipReason,
                                NoTunnel,
                                TlsDisabled,
                                BlockedByEarlierFailure,
                                NotApplicable
                            ),
                        },
                        StageResult::Failed {
                            elapsed_ms,
                            kind,
                            message,
                        } => NativeStageResult::Failed {
                            elapsed_ms,
                            message,
                            kind: same_enum!(
                                kind,
                                FailureKind,
                                NativeFailureKind,
                                TunnelFailed,
                                DnsUnresolvable,
                                ConnectionRefused,
                                TimedOut,
                                Unreachable,
                                ServerRefusedTls,
                                CertificateUntrusted,
                                HostnameMismatch,
                                ClientCertificateRejected,
                                InvalidLocalMaterial,
                                HandshakeFailed,
                                AuthenticationFailed,
                                DatabaseMissing,
                                Other
                            ),
                        },
                        StageResult::Passed { elapsed_ms, detail } => NativeStageResult::Passed {
                            elapsed_ms,
                            detail: detail.map(|detail| match detail {
                                StageDetail::Tunnel { local_endpoint } => {
                                    NativeStageDetail::Tunnel { local_endpoint }
                                }
                                StageDetail::Dns { addresses } => {
                                    NativeStageDetail::Dns { addresses }
                                }
                                StageDetail::Database { server_version } => {
                                    NativeStageDetail::Database { server_version }
                                }
                                StageDetail::Tls {
                                    encrypted,
                                    protocol,
                                    cipher,
                                    certificate_verified,
                                    hostname_verified,
                                    client_certificate_presented,
                                    pool_hostname_verification,
                                } => NativeStageDetail::Tls {
                                    encrypted,
                                    protocol,
                                    cipher,
                                    certificate_verified,
                                    hostname_verified,
                                    client_certificate_presented,
                                    pool_hostname_verification_ca_only: matches!(
                                        pool_hostname_verification,
                                        PoolHostnameVerification::CaOnly
                                    ),
                                },
                            }),
                        },
                    },
                })
                .collect(),
        }
    }
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.stages.len() != 6 || self.warnings.len() > 3 {
            return None;
        }
        let mut n = std::mem::size_of::<Self>()
            .checked_add(
                self.stages
                    .capacity()
                    .checked_mul(std::mem::size_of::<NativeDiagnosisStage>())?,
            )?
            .checked_add(
                self.warnings
                    .capacity()
                    .checked_mul(std::mem::size_of::<NativeDiagnosisWarning>())?,
            )?;
        fn text(n: &mut usize, s: &String, max: usize) -> Option<()> {
            if s.len() > max || s.capacity() > max * 2 {
                return None;
            }
            *n = n.checked_add(s.capacity())?;
            Some(())
        }
        for (index, stage) in self.stages.iter().enumerate() {
            if stage.stage != NATIVE_STAGE_ORDER[index] {
                return None;
            }
            match &stage.result {
                NativeStageResult::Failed { message, .. } => text(&mut n, message, 1024)?,
                NativeStageResult::Passed {
                    detail: Some(detail),
                    ..
                } => match detail {
                    NativeStageDetail::Tunnel { local_endpoint } => {
                        text(&mut n, local_endpoint, 256)?
                    }
                    NativeStageDetail::Dns { addresses } => {
                        if addresses.len() > 32 {
                            return None;
                        }
                        n = n.checked_add(
                            addresses
                                .capacity()
                                .checked_mul(std::mem::size_of::<String>())?,
                        )?;
                        for address in addresses {
                            text(&mut n, address, 64)?;
                        }
                    }
                    NativeStageDetail::Database { server_version } => {
                        text(&mut n, server_version, 1024)?
                    }
                    NativeStageDetail::Tls {
                        protocol, cipher, ..
                    } => {
                        for value in [protocol, cipher].into_iter().flatten() {
                            text(&mut n, value, 128)?;
                        }
                    }
                },
                _ => {}
            }
        }
        (n <= MAX_DIAGNOSIS_BYTES).then_some(n)
    }
}
fn stage_kind(stage: DiagnosisStageKind) -> NativeStageKind {
    same_enum!(
        stage,
        DiagnosisStageKind,
        NativeStageKind,
        Tunnel,
        Dns,
        Tcp,
        Tls,
        Authentication,
        Database
    )
}
