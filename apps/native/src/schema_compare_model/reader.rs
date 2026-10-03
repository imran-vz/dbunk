use super::*;
use std::collections::VecDeque;

#[derive(Clone, Copy)]
pub enum Turn {
    Next,
    Previous,
}
#[derive(Clone)]
pub enum Intent {
    Open(ResultRequest),
    ObjectPage(Turn),
    SelectObject(RelationIdentity),
    FieldPage(Turn),
    SelectField(FieldPath),
    ValuePage(Side, Turn),
    Retry,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ReadToken {
    owner: uuid::Uuid,
    epoch: u64,
    serial: u64,
}
pub struct Dispatch {
    pub token: ReadToken,
    pub request: ResultRequest,
    pub read: ReadRequest,
}
#[derive(Default)]
struct History {
    offsets: Vec<u32>,
}
#[derive(Clone, Copy)]
enum Move {
    Initial,
    Next { from: u32 },
    Previous { to: u32 },
}
impl History {
    fn plan(&self, offset: u32, next: Option<u32>, turn: Turn) -> Option<(u32, Move)> {
        match turn {
            Turn::Next => next
                .filter(|next| *next > offset)
                .map(|next| (next, Move::Next { from: offset })),
            Turn::Previous => self
                .offsets
                .last()
                .copied()
                .map(|to| (to, Move::Previous { to })),
        }
    }
    fn commit(&mut self, movement: Move) -> Result<(), &'static str> {
        match movement {
            Move::Initial => self.offsets.clear(),
            Move::Next { from } => {
                if self.offsets.len() >= 100_000 {
                    return Err("Comparison navigation history is full");
                }
                self.offsets.push(from);
            }
            Move::Previous { to } => {
                if self.offsets.last() != Some(&to) {
                    return Err("Comparison navigation history changed");
                }
                self.offsets.pop();
            }
        }
        Ok(())
    }
}
struct Step {
    read: ReadRequest,
    movement: Move,
}
struct Pending {
    token: ReadToken,
    step: Step,
}
pub struct ReaderState {
    owner: uuid::Uuid,
    epoch: u64,
    serial: u64,
    request: Option<ResultRequest>,
    pending: Option<Pending>,
    queued: Option<Intent>,
    last: Option<Intent>,
    steps: VecDeque<Step>,
    metadata: Option<PageCapture>,
    objects: Option<PageCapture>,
    fields: Option<PageCapture>,
    eligibility: [Option<PageCapture>; 2],
    values: [Option<PageCapture>; 2],
    selected_object: Option<RelationIdentity>,
    selected_field: Option<FieldPath>,
    object_history: History,
    field_history: History,
    value_history: [History; 2],
    _lease: Lease,
}
impl ReaderState {
    pub fn new(budget: Rc<Cell<usize>>) -> Result<Self, &'static str> {
        Ok(Self {
            owner: uuid::Uuid::new_v4(),
            epoch: 0,
            serial: 0,
            request: None,
            pending: None,
            queued: None,
            last: None,
            steps: VecDeque::new(),
            metadata: None,
            objects: None,
            fields: None,
            eligibility: [None, None],
            values: [None, None],
            selected_object: None,
            selected_field: None,
            object_history: Default::default(),
            field_history: Default::default(),
            value_history: Default::default(),
            _lease: Lease::new(budget, 1024 * 1024)?,
        })
    }
    pub fn request(&self) -> Option<&ResultRequest> {
        self.request.as_ref()
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some() || self.queued.is_some()
    }
    pub fn accepts(&self, token: &ReadToken) -> bool {
        token.epoch == self.epoch
            && self
                .pending
                .as_ref()
                .is_some_and(|pending| pending.token == *token)
    }
    pub fn metadata(&self) -> Option<&CompareReply> {
        self.metadata.as_ref().map(|capture| &capture.page.reply)
    }
    pub fn objects(&self) -> Option<&CompareReply> {
        self.objects.as_ref().map(|capture| &capture.page.reply)
    }
    pub fn fields(&self) -> Option<&CompareReply> {
        self.fields.as_ref().map(|capture| &capture.page.reply)
    }
    pub fn eligibility(&self, side: Side) -> Option<&Eligibility> {
        match &self.eligibility[side_index(side)].as_ref()?.page.reply {
            CompareReply::Eligibility { eligibility, .. } => Some(eligibility),
            _ => None,
        }
    }
    pub fn value(&self, side: Side) -> Option<&CompareReply> {
        self.values[side_index(side)]
            .as_ref()
            .map(|capture| &capture.page.reply)
    }
    pub fn selected_object(&self) -> Option<&RelationIdentity> {
        self.selected_object.as_ref()
    }
    pub fn selected_field(&self) -> Option<&FieldPath> {
        self.selected_field.as_ref()
    }
    pub fn selected_object_summary(&self) -> Option<&ObjectSummary> {
        let identity = self.selected_object.as_ref()?;
        let CompareReply::Objects { items, .. } = self.objects()? else {
            return None;
        };
        items.iter().find(|item| object_identity(item) == identity)
    }
    pub fn selected_field_summary(&self) -> Option<&FieldSummary> {
        let path = self.selected_field.as_ref()?;
        let CompareReply::Fields { items, .. } = self.fields()? else {
            return None;
        };
        items.iter().find(|item| &item.path == path)
    }
    pub fn selected_value(&self, side: Side) -> Option<ValueRef> {
        let field = self.selected_field_summary()?;
        let (source, target) = field_sides(field);
        match side {
            Side::Source => source,
            Side::Target => target,
        }
    }
    pub fn can_turn_objects(&self, turn: Turn) -> bool {
        self.objects().is_some_and(|reply|matches!(reply,CompareReply::Objects{offset,next_offset,..}if self.object_history.plan(*offset,*next_offset,turn).is_some()))
    }
    pub fn can_turn_fields(&self, turn: Turn) -> bool {
        self.fields().is_some_and(|reply|matches!(reply,CompareReply::Fields{offset,next_offset,..}if self.field_history.plan(*offset,*next_offset,turn).is_some()))
    }
    pub fn can_turn_value(&self, side: Side, turn: Turn) -> bool {
        self.value(side).is_some_and(|reply|matches!(reply,CompareReply::Value{offset,next_offset,complete,..}if self.value_history[side_index(side)].plan(*offset,(!complete).then_some(*next_offset),turn).is_some()))
    }
    pub fn enqueue(&mut self, intent: Intent) -> Result<Option<Dispatch>, &'static str> {
        match &intent {
            Intent::Open(request) if !valid_request(request) => {
                return Err("Comparison result endpoints are invalid");
            }
            Intent::SelectObject(value) if !valid_relation(value) => {
                return Err("Comparison object identity is invalid");
            }
            Intent::SelectField(value) if !valid_field(value) => {
                return Err("Comparison field identity is invalid");
            }
            _ => {}
        }
        self.epoch = self
            .epoch
            .checked_add(1)
            .ok_or("Comparison reader generation exhausted")?;
        self.queued = Some(intent);
        if self.pending.is_some() {
            return Ok(None);
        }
        self.advance()
    }
    fn clear_fields(&mut self) {
        self.fields = None;
        self.selected_field = None;
        self.values = [None, None];
        self.field_history = Default::default();
        self.value_history = Default::default();
    }
    fn clear_objects(&mut self) {
        self.objects = None;
        self.selected_object = None;
        self.eligibility = [None, None];
        self.object_history = Default::default();
        self.clear_fields();
    }
    fn clear_payloads(&mut self) {
        self.metadata = None;
        self.clear_objects();
    }
    fn push(&mut self, read: ReadRequest, movement: Move) {
        self.steps.push_back(Step { read, movement });
    }
    fn start_intent(&mut self, mut intent: Intent) -> Result<(), &'static str> {
        if matches!(intent, Intent::Retry) {
            intent = if self.metadata.is_none() {
                Intent::Open(
                    self.request
                        .clone()
                        .ok_or("No comparison result to retry")?,
                )
            } else {
                self.last.clone().ok_or("No comparison read to retry")?
            };
        }
        self.last = Some(intent.clone());
        self.steps.clear();
        match intent {
            Intent::Open(request) => {
                self.clear_payloads();
                self.request = Some(request);
                self.push(ReadRequest::Metadata, Move::Initial);
                self.push(ReadRequest::Objects { offset: 0 }, Move::Initial);
            }
            Intent::ObjectPage(turn) => {
                if let Some(CompareReply::Objects {
                    offset,
                    next_offset,
                    ..
                }) = self.objects()
                    && let Some((offset, movement)) =
                        self.object_history.plan(*offset, *next_offset, turn)
                {
                    self.push(ReadRequest::Objects { offset }, movement);
                }
            }
            Intent::SelectObject(identity) => {
                let Some(CompareReply::Objects { items, .. }) = self.objects() else {
                    return Err("Load the current object page first");
                };
                let item = items
                    .iter()
                    .find(|item| object_identity(item) == &identity)
                    .ok_or("Selected object is no longer on this page")?;
                let count = item.field_count;
                let excluded = matches!(
                    item.difference,
                    SummaryDifference::NotComparable {
                        reason: IncomparableReason::ExcludedObject
                            | IncomparableReason::ExcludedCounterpart,
                        ..
                    }
                );
                let sides = object_sides(item);
                let source = sides.0.cloned();
                let target = sides.1.cloned();
                self.clear_fields();
                self.eligibility = [None, None];
                self.selected_object = Some(identity.clone());
                if count > 0 {
                    self.push(
                        ReadRequest::Fields {
                            object: identity,
                            offset: 0,
                        },
                        Move::Initial,
                    );
                }
                if excluded {
                    if let Some(object) = source {
                        self.push(
                            ReadRequest::Eligibility {
                                object,
                                side: Side::Source,
                            },
                            Move::Initial,
                        );
                    }
                    if let Some(object) = target {
                        self.push(
                            ReadRequest::Eligibility {
                                object,
                                side: Side::Target,
                            },
                            Move::Initial,
                        );
                    }
                }
            }
            Intent::FieldPage(turn) => {
                if let Some(CompareReply::Fields {
                    object,
                    offset,
                    next_offset,
                    ..
                }) = self.fields()
                    && let Some((offset, movement)) =
                        self.field_history.plan(*offset, *next_offset, turn)
                {
                    self.push(
                        ReadRequest::Fields {
                            object: object.clone(),
                            offset,
                        },
                        movement,
                    );
                }
            }
            Intent::SelectField(path) => {
                let Some(CompareReply::Fields { items, .. }) = self.fields() else {
                    return Err("Load the current field page first");
                };
                let field = items
                    .iter()
                    .find(|field| field.path == path)
                    .ok_or("Selected field is no longer on this page")?;
                let (source, target) = field_sides(field);
                self.selected_field = Some(path);
                self.values = [None, None];
                self.value_history = Default::default();
                for value in [source, target].into_iter().flatten() {
                    if value.value_kind != ValueKind::Null && value.raw_bytes != 0 {
                        self.push(ReadRequest::Value { value, offset: 0 }, Move::Initial);
                    }
                }
            }
            Intent::ValuePage(side, turn) => {
                if let Some(CompareReply::Value {
                    value,
                    offset,
                    next_offset,
                    complete,
                    ..
                }) = self.value(side)
                    && let Some((offset, movement)) = self.value_history[side_index(side)].plan(
                        *offset,
                        (!complete).then_some(*next_offset),
                        turn,
                    )
                {
                    self.push(
                        ReadRequest::Value {
                            value: *value,
                            offset,
                        },
                        movement,
                    );
                }
            }
            Intent::Retry => unreachable!(),
        }
        Ok(())
    }
    fn advance(&mut self) -> Result<Option<Dispatch>, &'static str> {
        if let Some(intent) = self.queued.take() {
            self.start_intent(intent)?;
        }
        let Some(step) = self.steps.pop_front() else {
            return Ok(None);
        };
        let request = self
            .request
            .clone()
            .ok_or("Comparison result is unavailable")?;
        self.serial = self
            .serial
            .checked_add(1)
            .ok_or("Comparison read identity exhausted")?;
        let token = ReadToken {
            owner: self.owner,
            epoch: self.epoch,
            serial: self.serial,
        };
        let read = step.read.clone();
        self.pending = Some(Pending { token, step });
        Ok(Some(Dispatch {
            token,
            request,
            read,
        }))
    }
    pub fn accept(
        &mut self,
        token: ReadToken,
        page: SchemaComparisonPage,
        lease: PageLease,
    ) -> Result<Option<Dispatch>, &'static str> {
        if self
            .pending
            .as_ref()
            .is_none_or(|pending| pending.token != token)
        {
            return Ok(None);
        }
        let pending = self.pending.take().unwrap();
        if token.epoch != self.epoch {
            drop(page);
            drop(lease);
            self.steps.clear();
            return self.advance();
        }
        if !lease.admits(&page)
            || self.request.as_ref() != Some(&page.request)
            || !same_read(&pending.step.read, &page.read)
            || !reply_matches(&pending.step.read, &page.reply)
        {
            self.steps.clear();
            return Err("Comparison response identity or payload is invalid");
        }
        let capture = PageCapture {
            page,
            _lease: lease,
        };
        match &capture.page.reply {
            CompareReply::Metadata { .. } => self.metadata = Some(capture),
            CompareReply::Objects { .. } => {
                if let Err(error) = self.object_history.commit(pending.step.movement) {
                    self.steps.clear();
                    return Err(error);
                }
                self.selected_object = None;
                self.eligibility = [None, None];
                self.clear_fields();
                self.objects = Some(capture);
            }
            CompareReply::Fields { .. } => {
                if let Err(error) = self.field_history.commit(pending.step.movement) {
                    self.steps.clear();
                    return Err(error);
                }
                self.selected_field = None;
                self.values = [None, None];
                self.value_history = Default::default();
                self.fields = Some(capture);
            }
            CompareReply::Eligibility { side, .. } => {
                let index = side_index(*side);
                self.eligibility[index] = Some(capture);
            }
            CompareReply::Value { value, .. } => {
                let index = side_index(value.side);
                if let Err(error) = self.value_history[index].commit(pending.step.movement) {
                    self.steps.clear();
                    return Err(error);
                }
                self.values[index] = Some(capture);
            }
        }
        self.advance()
    }
    /// The host always consumes the delivered response lease, even when this
    /// method discards stale UI authority. Unavailable is never an empty result.
    pub fn fail(
        &mut self,
        token: ReadToken,
        unavailable: bool,
    ) -> Result<Option<Dispatch>, &'static str> {
        if self
            .pending
            .as_ref()
            .is_none_or(|pending| pending.token != token)
        {
            return Ok(None);
        }
        self.pending = None;
        self.steps.clear();
        if token.epoch == self.epoch && unavailable {
            self.clear_payloads();
        }
        self.advance()
    }
    pub fn close(&mut self) {
        self.owner = uuid::Uuid::new_v4();
        self.epoch = 0;
        self.pending = None;
        self.queued = None;
        self.last = None;
        self.steps.clear();
        self.request = None;
        self.clear_payloads();
    }
}
pub(super) fn side_index(side: Side) -> usize {
    match side {
        Side::Source => 0,
        Side::Target => 1,
    }
}
pub(super) fn object_sides(
    item: &ObjectSummary,
) -> (Option<&RelationIdentity>, Option<&RelationIdentity>) {
    match &item.difference {
        SummaryDifference::Equal { source, target }
        | SummaryDifference::Changed { source, target } => (Some(source), Some(target)),
        SummaryDifference::SourceOnly { source } => (Some(source), None),
        SummaryDifference::TargetOnly { target } => (None, Some(target)),
        SummaryDifference::NotComparable { observed, .. } => match observed {
            ObservedSides::Both { source, target } => (Some(source), Some(target)),
            ObservedSides::Source { source } => (Some(source), None),
            ObservedSides::Target { target } => (None, Some(target)),
        },
    }
}
pub(super) fn field_sides(item: &FieldSummary) -> (Option<ValueRef>, Option<ValueRef>) {
    match item.difference {
        SummaryDifference::Equal { source, target }
        | SummaryDifference::Changed { source, target } => (Some(source), Some(target)),
        SummaryDifference::SourceOnly { source } => (Some(source), None),
        SummaryDifference::TargetOnly { target } => (None, Some(target)),
        SummaryDifference::NotComparable { observed, .. } => match observed {
            ObservedSides::Both { source, target } => (Some(source), Some(target)),
            ObservedSides::Source { source } => (Some(source), None),
            ObservedSides::Target { target } => (None, Some(target)),
        },
    }
}
fn same_read(a: &ReadRequest, b: &ReadRequest) -> bool {
    match (a, b) {
        (ReadRequest::Metadata, ReadRequest::Metadata) => true,
        (ReadRequest::Objects { offset: a }, ReadRequest::Objects { offset: b }) => a == b,
        (
            ReadRequest::Fields {
                object: a,
                offset: x,
            },
            ReadRequest::Fields {
                object: b,
                offset: y,
            },
        ) => a == b && x == y,
        (
            ReadRequest::Eligibility { object: a, side: x },
            ReadRequest::Eligibility { object: b, side: y },
        ) => a == b && x == y,
        (
            ReadRequest::Value {
                value: a,
                offset: x,
            },
            ReadRequest::Value {
                value: b,
                offset: y,
            },
        ) => a == b && x == y,
        _ => false,
    }
}
fn reply_matches(read: &ReadRequest, reply: &CompareReply) -> bool {
    match (read, reply) {
        (ReadRequest::Metadata, CompareReply::Metadata { .. }) => true,
        (
            ReadRequest::Objects { offset: a },
            CompareReply::Objects {
                offset: b,
                next_offset,
                items,
            },
        ) => {
            a == b
                && *b <= 4000
                && next_offset.is_none_or(|next| next <= 4000)
                && valid_page(*b, *next_offset, items.len())
        }
        (
            ReadRequest::Fields {
                object: a,
                offset: x,
            },
            CompareReply::Fields {
                object: b,
                offset: y,
                next_offset,
                items,
            },
        ) => a == b && x == y && valid_page(*y, *next_offset, items.len()),
        (
            ReadRequest::Eligibility { object: a, side: x },
            CompareReply::Eligibility {
                object: b, side: y, ..
            },
        ) => a == b && x == y,
        (
            ReadRequest::Value {
                value: a,
                offset: x,
            },
            CompareReply::Value {
                value: b,
                offset: y,
                text,
                next_offset,
                complete,
            },
        ) => {
            a == b
                && x == y
                && b.raw_bytes <= 256 * 1024
                && text.len() <= 64 * 1024
                && (*y as usize).checked_add(text.len()) == Some(*next_offset as usize)
                && *next_offset <= b.raw_bytes
                && (*complete == (*next_offset == b.raw_bytes))
                && (*complete || (next_offset > y && text.len() >= 64 * 1024 - 3))
        }
        _ => false,
    }
}
fn valid_page(offset: u32, next: Option<u32>, count: usize) -> bool {
    count <= 100
        && offset <= 100_000
        && next.is_none_or(|next| {
            next > offset && next <= 100_000 && offset.checked_add(count as u32) == Some(next)
        })
}
