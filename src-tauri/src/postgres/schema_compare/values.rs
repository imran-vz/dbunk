use super::{budget::*, protocol::*};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Write},
    mem::size_of,
    time::{Duration, Instant},
};

pub const RESULT_TTL: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ValueKind {
    Null,
    Text,
    Boolean,
    Integer,
    QualifiedName,
    OrderedNames,
    OrderedReferences,
    OperatorSignatures,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueRef {
    pub side: Side,
    pub value_id: u32,
    pub raw_bytes: u32,
    pub value_kind: ValueKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueRequest {
    pub identity: ResultIdentity,
    pub value: ValueRef,
    pub offset: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Chunk<'a> {
    response_id: &'a str,
    identity: &'a ResultIdentity,
    value: &'a ValueRef,
    offset: u32,
    text: &'a str,
    next_offset: u32,
    complete: bool,
}

struct Blob {
    text: Box<str>,
    side: Side,
    kind: ValueKind,
    _reservation: Reservation,
}

/// One immutable retained copy per value. Fixed slot storage is charged before
/// allocation, as are each blob and the identity strings. References never
/// clone definition text. The owner must drop this store on endpoint teardown.
pub struct Values {
    identity: ResultIdentity,
    created: Instant,
    blobs: Box<[Option<Blob>]>,
    count: usize,
    retained_bytes: usize,
    side_bytes: [usize; 2],
    side_values: [usize; 2],
    budget: Budget,
    _reservation: Reservation,
}

impl Values {
    pub fn new(
        identity: &ResultIdentity,
        budget: &Budget,
        created: Instant,
    ) -> Result<Self, CompareError> {
        identity.validate()?;
        let budget = budget.result_scope();
        let retained_bytes = MAX_RESULT_VALUES * size_of::<Option<Blob>>()
            + size_of::<Self>()
            + identity.job_id.len()
            + identity.result_id.len();
        let reservation = budget.reserve(retained_bytes)?;
        let blobs = std::iter::repeat_with(|| None)
            .take(MAX_RESULT_VALUES)
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Ok(Self {
            identity: identity.clone(),
            created,
            blobs,
            count: 0,
            retained_bytes,
            side_bytes: [0, 0],
            side_values: [0, 0],
            budget: budget.clone(),
            _reservation: reservation,
        })
    }

    pub fn result_budget(&self) -> Budget {
        self.budget.clone()
    }

    fn insert(
        &mut self,
        side: Side,
        kind: ValueKind,
        text: &str,
    ) -> Result<ValueRef, CompareError> {
        if text.len() > FIELD_BYTES {
            return Err(CompareError::LimitExceeded {
                limit: Limit::FieldBytes,
            });
        }
        let side_index = if side == Side::Source { 0 } else { 1 };
        if self.side_values[side_index] == MAX_VALUES {
            return Err(CompareError::LimitExceeded {
                limit: Limit::ChildFacts,
            });
        }
        let side_size = self.side_bytes[side_index] + text.len() + size_of::<Option<Blob>>();
        if side_size > ENDPOINT_BYTES {
            return Err(CompareError::LimitExceeded {
                limit: Limit::EndpointBytes,
            });
        }
        if self.retained_bytes + text.len() > RESULT_BYTES {
            return Err(CompareError::LimitExceeded {
                limit: Limit::ResultBytes,
            });
        }
        let reservation = self.budget.reserve(text.len())?;
        let reference = ValueRef {
            side,
            value_id: self.count as u32,
            raw_bytes: text.len() as u32,
            value_kind: kind,
        };
        self.blobs[self.count] = Some(Blob {
            text: text.into(),
            side,
            kind,
            _reservation: reservation,
        });
        self.count += 1;
        self.retained_bytes += text.len();
        self.side_bytes[side_index] = side_size;
        self.side_values[side_index] += 1;
        Ok(reference)
    }

    /// Typed input prevents the value kind from disagreeing with its content.
    /// Text remains raw UTF-8; structured facts use compact JSON in the blob.
    pub fn insert_fact(
        &mut self,
        side: Side,
        fact: super::normalize::Fact<'_>,
    ) -> Result<ValueRef, CompareError> {
        use super::normalize::Fact;
        if let Fact::Text(text)
        | Fact::NotComparable {
            raw: Some(text), ..
        } = fact
        {
            return self.insert(side, ValueKind::Text, text);
        }
        let kind = match fact {
            Fact::Null => ValueKind::Null,
            Fact::Boolean(_) => ValueKind::Boolean,
            Fact::Integer(_) => ValueKind::Integer,
            Fact::Reference(_) => ValueKind::QualifiedName,
            Fact::Names(_) => ValueKind::OrderedNames,
            Fact::References(_) => ValueKind::OrderedReferences,
            Fact::Operators(_) => ValueKind::OperatorSignatures,
            Fact::Text(_) | Fact::NotComparable { .. } => return Err(CompareError::InvalidRequest),
        };
        // The temporary representation is reserved independently while the
        // immutable blob is copied. It never borrows a page serializer slot.
        let _scratch = self.budget.scratch(PAGE_BYTES)?;
        let mut writer = CappedWriter {
            bytes: Vec::with_capacity(PAGE_BYTES),
        };
        let encoded = match fact {
            Fact::Null => serde_json::to_writer(&mut writer, &()),
            Fact::Boolean(value) => serde_json::to_writer(&mut writer, &value),
            Fact::Integer(value) => serde_json::to_writer(&mut writer, &value),
            Fact::Reference(value) => serde_json::to_writer(&mut writer, value),
            Fact::Names(value) => serde_json::to_writer(&mut writer, value),
            Fact::References(value) => serde_json::to_writer(&mut writer, value),
            Fact::Operators(value) => serde_json::to_writer(&mut writer, value),
            _ => return Err(CompareError::InvalidRequest),
        };
        encoded.map_err(|_| CompareError::LimitExceeded {
            limit: Limit::FieldBytes,
        })?;
        let text = std::str::from_utf8(&writer.bytes).map_err(|_| CompareError::InvalidRequest)?;
        self.insert(side, kind, text)
    }

    pub fn chunk(
        &self,
        request: &ValueRequest,
        response_id: &str,
        now: Instant,
    ) -> Result<EncodedPage, CompareError> {
        if now.saturating_duration_since(self.created) >= RESULT_TTL
            || request.identity != self.identity
        {
            return Err(CompareError::Unavailable);
        }
        let blob = self
            .blobs
            .get(request.value.value_id as usize)
            .and_then(Option::as_ref)
            .ok_or(CompareError::Unavailable)?;
        if blob.side != request.value.side
            || blob.kind != request.value.value_kind
            || blob.text.len() != request.value.raw_bytes as usize
        {
            return Err(CompareError::Unavailable);
        }
        let start = request.offset as usize;
        if start > blob.text.len() || !blob.text.is_char_boundary(start) {
            return Err(CompareError::InvalidRequest);
        }
        let mut end = (start + CHUNK_BYTES).min(blob.text.len());
        while !blob.text.is_char_boundary(end) {
            end -= 1;
        }
        validate_response_id(response_id)?;
        let lease = self.budget.serializer()?;
        encode(
            &Chunk {
                response_id,
                identity: &self.identity,
                value: &request.value,
                offset: request.offset,
                text: &blob.text[start..end],
                next_offset: end as u32,
                complete: end == blob.text.len(),
            },
            response_id,
            lease,
        )
    }
}

/// Owns serializer scratch until its body has been handed off and acknowledged.
/// This deliberately does NOT implement Serialize or Tauri IpcResponse: those
/// drop their source value before the transport has consumed its buffers.
pub struct EncodedPage {
    json: String,
    lease: SerializerLease,
    response_id: Box<str>,
}

impl EncodedPage {
    pub fn as_str(&self) -> &str {
        &self.json
    }
    pub fn bytes(&self) -> usize {
        self.json.len()
    }
}

/// A capped writer checks BEFORE extending its reserved buffer. It includes
/// JSON escaping and envelope bytes, rather than checking a finished response.
struct CappedWriter {
    bytes: Vec<u8>,
}

impl Write for CappedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > PAGE_BYTES - self.bytes.len() {
            return Err(io::Error::other("schema comparison page limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn encode(
    value: &impl Serialize,
    response_id: &str,
    lease: SerializerLease,
) -> Result<EncodedPage, CompareError> {
    validate_response_id(response_id)?;
    let mut writer = CappedWriter {
        bytes: Vec::with_capacity(PAGE_BYTES),
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| CompareError::LimitExceeded {
        limit: Limit::PageBytes,
    })?;
    let json = String::from_utf8(writer.bytes).map_err(|_| CompareError::InvalidRequest)?;
    Ok(EncodedPage {
        json,
        lease,
        response_id: response_id.into(),
    })
}

pub(crate) fn validate_response_id(response_id: &str) -> Result<(), CompareError> {
    if response_id.is_empty() || response_id.len() > 128 {
        return Err(CompareError::InvalidRequest);
    }
    Ok(())
}

struct InFlight {
    session: String,
    response_id: String,
    _lease: SerializerLease,
}

/// Integration boundary for Step 5: insert before handing the body to Tauri,
/// acknowledge only after that webview has received it. Never free a slot on
/// timeout, job release, or an uncertain send result. A confirmed destroyed
/// transport can release its session. Two unacknowledged replies cause busy.
#[derive(Default)]
pub struct ResponseOwnership {
    slots: [Option<InFlight>; 2],
}

impl ResponseOwnership {
    pub fn handoff(
        &mut self,
        session: &str,
        page: EncodedPage,
        send: impl FnOnce(String),
    ) -> Result<(), CompareError> {
        if session.is_empty() || session.len() > 128 {
            return Err(CompareError::InvalidRequest);
        }
        if self
            .slots
            .iter()
            .flatten()
            .any(|slot| slot.response_id == page.response_id.as_ref())
        {
            return Err(CompareError::InvalidRequest);
        }
        let slot = self
            .slots
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(CompareError::Busy)?;
        let EncodedPage {
            json,
            lease,
            response_id,
        } = page;
        *slot = Some(InFlight {
            session: session.into(),
            response_id: response_id.into(),
            _lease: lease,
        });
        send(json);
        Ok(())
    }

    pub fn acknowledge(&mut self, session: &str, response_id: &str) -> Result<(), CompareError> {
        let slot = self
            .slots
            .iter_mut()
            .find(|slot| {
                slot.as_ref()
                    .is_some_and(|slot| slot.session == session && slot.response_id == response_id)
            })
            .ok_or(CompareError::Unavailable)?;
        *slot = None;
        Ok(())
    }

    pub fn transport_destroyed(&mut self, session: &str) {
        for slot in &mut self.slots {
            if slot.as_ref().is_some_and(|slot| slot.session == session) {
                *slot = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> ResultIdentity {
        ResultIdentity {
            job_id: "job".into(),
            result_id: "result".into(),
        }
    }

    #[test]
    fn maximum_control_text_uses_four_independent_bounded_chunks() {
        let budget = Budget::default();
        let now = Instant::now();
        let mut values = Values::new(&identity(), &budget, now).unwrap();
        let text = "\u{1}".repeat(FIELD_BYTES);
        let source = values.insert(Side::Source, ValueKind::Text, &text).unwrap();
        let target = values
            .insert(Side::Target, ValueKind::Text, "different")
            .unwrap();
        assert_ne!(source.value_id, target.value_id);
        for offset in (0..FIELD_BYTES).step_by(CHUNK_BYTES) {
            let page = values
                .chunk(
                    &ValueRequest {
                        identity: identity(),
                        value: source,
                        offset: offset as u32,
                    },
                    "response",
                    now,
                )
                .unwrap();
            assert!(page.bytes() < PAGE_BYTES);
            let json: serde_json::Value = serde_json::from_str(page.as_str()).unwrap();
            assert_eq!(json["text"].as_str().unwrap().len(), CHUNK_BYTES);
            assert_eq!(json["nextOffset"], offset + CHUNK_BYTES);
        }
        assert!(values
            .insert(Side::Source, ValueKind::Text, &"x".repeat(FIELD_BYTES + 1))
            .is_err());
    }

    #[test]
    fn utf8_offsets_result_identity_and_expiry_are_checked() {
        let budget = Budget::default();
        let now = Instant::now();
        let mut values = Values::new(&identity(), &budget, now).unwrap();
        let value = values
            .insert(Side::Source, ValueKind::Text, &"€".repeat(CHUNK_BYTES))
            .unwrap();
        let request = ValueRequest {
            identity: identity(),
            value,
            offset: 0,
        };
        let page = values.chunk(&request, "response", now).unwrap();
        let json: serde_json::Value = serde_json::from_str(page.as_str()).unwrap();
        assert_eq!(json["nextOffset"], 65535);
        let mut invalid = request.clone();
        invalid.offset = 1;
        assert!(matches!(
            values.chunk(&invalid, "response", now),
            Err(CompareError::InvalidRequest)
        ));
        invalid = request.clone();
        invalid.identity.result_id = "other".into();
        assert!(matches!(
            values.chunk(&invalid, "response", now),
            Err(CompareError::Unavailable)
        ));
        assert!(matches!(
            values.chunk(&request, "response", now + RESULT_TTL),
            Err(CompareError::Unavailable)
        ));
    }

    #[test]
    fn handoff_retains_both_serializer_reservations_until_matching_ack() {
        let budget = Budget::default();
        let mut ownership = ResponseOwnership::default();
        for id in ["one", "two"] {
            let page = encode(&"payload", id, budget.serializer().unwrap()).unwrap();
            ownership
                .handoff("webview", page, |body| assert_eq!(body, "\"payload\""))
                .unwrap();
        }
        assert!(matches!(budget.serializer(), Err(CompareError::Busy)));
        assert_eq!(budget.used(), 2 * SERIALIZER_SCRATCH);
        assert_eq!(
            ownership.acknowledge("other", "one"),
            Err(CompareError::Unavailable)
        );
        ownership.acknowledge("webview", "one").unwrap();
        assert!(budget.serializer().is_ok());
        ownership.transport_destroyed("webview");
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn encoded_pages_require_valid_response_ids_and_handoff_uses_the_bound_id() {
        let budget = Budget::default();
        let oversized = "x".repeat(129);
        for response_id in ["", oversized.as_str()] {
            assert!(matches!(
                encode(&"payload", response_id, budget.serializer().unwrap()),
                Err(CompareError::InvalidRequest)
            ));
            assert_eq!(budget.used(), 0);
        }

        let page = encode(&"payload", "bound", budget.serializer().unwrap()).unwrap();
        let mut ownership = ResponseOwnership::default();
        ownership
            .handoff("webview", page, |body| assert_eq!(body, "\"payload\""))
            .unwrap();
        assert_eq!(
            ownership.acknowledge("webview", "other"),
            Err(CompareError::Unavailable)
        );
        ownership.acknowledge("webview", "bound").unwrap();
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn serialized_json_limit_is_checked_before_buffer_growth() {
        let budget = Budget::default();
        assert!(encode(
            &"x".repeat(PAGE_BYTES - 2),
            "response",
            budget.serializer().unwrap()
        )
        .is_ok());
        assert!(matches!(
            encode(
                &"x".repeat(PAGE_BYTES - 1),
                "response",
                budget.serializer().unwrap()
            ),
            Err(CompareError::LimitExceeded {
                limit: Limit::PageBytes
            })
        ));
        assert!(encode(
            &"\u{1}".repeat(PAGE_BYTES / 6 + 1),
            "response",
            budget.serializer().unwrap()
        )
        .is_err());
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn tauri_json_callback_duplication_fits_the_reserved_scratch() {
        // Exercise the exact dependency used by Tauri 2.11.2's
        // format_callback::serialize_js_with, including worst-case JS escaping.
        use serialize_to_javascript::{Options, RawValue, Serialized};
        let budget = Budget::default();
        for character in ['\'', '\\', '\u{1}', '€'] {
            let encoded_character_bytes = serde_json::to_string(&character).unwrap().len() - 2;
            let text = character
                .to_string()
                .repeat((PAGE_BYTES - 64) / encoded_character_bytes);
            let body = serde_json::json!({ "text": text });
            let page = encode(&body, "response", budget.serializer().unwrap()).unwrap();
            let mut ownership = ResponseOwnership::default();
            ownership
                .handoff("webview", page, |json| {
                    let raw = RawValue::from_string(json).unwrap();
                    let javascript = Serialized::new(&raw, &Options::default()).into_string();
                    let callback =
                        format!("window.__TAURI_INTERNALS__.runCallback(4294967295, {javascript})");
                    assert!(
                        raw.get().len() + javascript.capacity() + callback.capacity()
                            < SERIALIZER_SCRATCH
                    );
                    assert_eq!(budget.used(), SERIALIZER_SCRATCH);
                })
                .unwrap();
            assert_eq!(budget.used(), SERIALIZER_SCRATCH);
            ownership.acknowledge("webview", "response").unwrap();
        }
    }

    #[test]
    fn endpoint_and_combined_result_bytes_reject_before_retention() {
        let budget = Budget::default();
        let mut values = Values::new(&identity(), &budget, Instant::now()).unwrap();
        let field = "x".repeat(FIELD_BYTES);
        while ENDPOINT_BYTES - values.side_bytes[0] > FIELD_BYTES + size_of::<Option<Blob>>() {
            values
                .insert(Side::Source, ValueKind::Text, &field)
                .unwrap();
        }
        let remaining = ENDPOINT_BYTES - values.side_bytes[0] - size_of::<Option<Blob>>();
        values
            .insert(Side::Source, ValueKind::Text, &field[..remaining])
            .unwrap();
        assert!(matches!(
            values.insert(Side::Source, ValueKind::Text, ""),
            Err(CompareError::LimitExceeded {
                limit: Limit::EndpointBytes
            })
        ));
        while values.retained_bytes < RESULT_BYTES {
            let remaining = (RESULT_BYTES - values.retained_bytes).min(FIELD_BYTES);
            values
                .insert(Side::Target, ValueKind::Text, &field[..remaining])
                .unwrap();
        }
        assert!(matches!(
            values.insert(Side::Target, ValueKind::Text, "x"),
            Err(CompareError::LimitExceeded {
                limit: Limit::ResultBytes
            })
        ));
        drop(values);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn child_fact_slots_are_bounded_independently_for_each_endpoint() {
        let budget = Budget::default();
        let mut values = Values::new(&identity(), &budget, Instant::now()).unwrap();
        for side in [Side::Source, Side::Target] {
            for _ in 0..MAX_VALUES {
                values.insert(side, ValueKind::Text, "").unwrap();
            }
            assert!(matches!(
                values.insert(side, ValueKind::Text, ""),
                Err(CompareError::LimitExceeded {
                    limit: Limit::ChildFacts
                })
            ));
        }
        assert_eq!(values.count, MAX_RESULT_VALUES);
    }
}
