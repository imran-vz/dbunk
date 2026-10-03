use super::*;
/// Consecutive pages concatenate to the exact selected details. A very long
/// logical line may continue on the next page; no character is elided.
pub struct DetailPage {
    pub text: String,
    pub page: usize,
    pub next: bool,
}
struct PageWriter {
    selected: usize,
    current: usize,
    bytes: usize,
    lines: usize,
    selected_bytes: usize,
    text: Option<String>,
}
impl Write for PageWriter {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        for ch in value.chars() {
            let newline = usize::from(matches!(ch, '\n' | '\r'));
            if self.bytes + ch.len_utf8() > MAX_DETAIL_BYTES
                || self.lines + newline >= MAX_DETAIL_LINES
            {
                self.current += 1;
                self.bytes = 0;
                self.lines = 0;
            }
            self.bytes += ch.len_utf8();
            self.lines += newline;
            if self.current == self.selected {
                self.selected_bytes += ch.len_utf8();
                if let Some(text) = &mut self.text {
                    text.push(ch);
                }
            }
        }
        Ok(())
    }
}
impl Scene {
    pub fn details_page(
        &self,
        selection: Selection,
        page: usize,
    ) -> Result<DetailPage, &'static str> {
        if !self.accepts(selection) {
            return Err("Map selection belongs to an older capture or layout");
        }
        let mut measure = PageWriter {
            selected: page,
            current: 0,
            bytes: 0,
            lines: 0,
            selected_bytes: 0,
            text: None,
        };
        self.write_details(selection, &mut measure)
            .map_err(|_| "Invalid map detail metadata")?;
        if page > measure.current {
            return Err("Map detail page no longer exists");
        }
        let next = page < measure.current;
        let mut out = PageWriter {
            text: Some(String::with_capacity(measure.selected_bytes)),
            ..PageWriter {
                selected: page,
                current: 0,
                bytes: 0,
                lines: 0,
                selected_bytes: 0,
                text: None,
            }
        };
        self.write_details(selection, &mut out)
            .map_err(|_| "Map detail formatting failed")?;
        Ok(DetailPage {
            text: out.text.unwrap(),
            page,
            next,
        })
    }
}
