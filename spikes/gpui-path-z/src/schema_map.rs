//! Probe for a part with no ready-made component: a schema map with pan,
//! zoom, draggable table nodes and relationship edges. Nodes are ordinary
//! elements placed in screen space; edges are stroked paths on a canvas
//! underneath them. GPUI has no element transform, so zoom scales every
//! length and the font size by hand.

use gpui::{
    Context, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point,
    ScrollWheelEvent, SharedString, Window, canvas, div, point, prelude::*, px,
};
use theme::ActiveTheme;

const NODE_WIDTH: f32 = 180.;
const HEADER_HEIGHT: f32 = 26.;
const COLUMN_HEIGHT: f32 = 20.;
const COLUMNS_PER_TABLE: usize = 6;

struct Node {
    name: SharedString,
    columns: Vec<SharedString>,
    /// Top-left corner in map units, before pan and zoom.
    position: Point<f32>,
}

impl Node {
    fn height(&self) -> f32 {
        HEADER_HEIGHT + COLUMN_HEIGHT * self.columns.len() as f32
    }
}

enum Drag {
    Canvas { last: Point<Pixels> },
    Node { index: usize, last: Point<Pixels> },
}

pub struct SchemaMap {
    nodes: Vec<Node>,
    edges: Vec<(usize, usize)>,
    pan: Point<Pixels>,
    zoom: f32,
    drag: Option<Drag>,
}

impl SchemaMap {
    /// `tables` nodes on a grid, each related to a few earlier ones.
    pub fn new(tables: usize) -> Self {
        let per_row = (tables as f32).sqrt().ceil() as usize;
        let nodes = (0..tables)
            .map(|index| Node {
                name: format!("table_{index:03}").into(),
                columns: (0..COLUMNS_PER_TABLE)
                    .map(|column| match column {
                        0 => "id bigint".into(),
                        1 => "created_at timestamptz".into(),
                        _ => format!("column_{column} text").into(),
                    })
                    .collect(),
                position: point(
                    40. + (index % per_row) as f32 * 260.,
                    40. + (index / per_row) as f32 * 200.,
                ),
            })
            .collect();
        let mut edges = Vec::new();
        for index in 1..tables {
            edges.push((index, index / 2));
            if index % 3 == 0 {
                edges.push((index, index - 1));
            }
        }
        Self {
            nodes,
            edges,
            pan: point(px(0.), px(0.)),
            zoom: 1.,
            drag: None,
        }
    }

    fn scroll(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(window.line_height());
        if std::env::var_os("DBUNK_SPIKE_TRACE").is_some() {
            eprintln!("scroll {:?} platform={}", delta, event.modifiers.platform);
        }
        if event.modifiers.platform {
            // Zoom about the pointer: the map point under it stays put.
            let before = self.zoom;
            let delta_y: f32 = delta.y.into();
            self.zoom = (self.zoom * (1. + delta_y / 400.)).clamp(0.2, 3.);
            let scale = self.zoom / before;
            self.pan = point(
                event.position.x - (event.position.x - self.pan.x) * scale,
                event.position.y - (event.position.y - self.pan.y) * scale,
            );
        } else {
            self.pan += delta;
        }
        cx.notify();
    }

    fn drag(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let zoom = self.zoom;
        match &mut self.drag {
            Some(Drag::Canvas { last }) => {
                self.pan += event.position - *last;
                *last = event.position;
            }
            Some(Drag::Node { index, last }) => {
                let moved = event.position - *last;
                let (x, y): (f32, f32) = (moved.x.into(), moved.y.into());
                self.nodes[*index].position += point(x / zoom, y / zoom);
                *last = event.position;
            }
            None => return,
        }
        cx.notify();
    }
}

impl Render for SchemaMap {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let (pan, zoom) = (self.pan, self.zoom);
        let viewport = window.viewport_size();
        let edge_color = colors.text_muted;
        let to_screen =
            move |map: Point<f32>| point(pan.x + px(map.x * zoom), pan.y + px(map.y * zoom));

        // Edge endpoints are resolved now, in screen space relative to the
        // map's origin; the canvas adds its own origin when it paints.
        let segments: Vec<(Point<Pixels>, Point<Pixels>)> = self
            .edges
            .iter()
            .map(|(from, to)| {
                let (from, to) = (&self.nodes[*from], &self.nodes[*to]);
                (
                    to_screen(from.position + point(0., HEADER_HEIGHT / 2.)),
                    to_screen(to.position + point(NODE_WIDTH, HEADER_HEIGHT / 2.)),
                )
            })
            .collect();

        // Only nodes that intersect the viewport become elements.
        let visible = self.nodes.iter().enumerate().filter_map(|(index, node)| {
            let origin = to_screen(node.position);
            let (width, height) = (px(NODE_WIDTH * zoom), px(node.height() * zoom));
            let hidden = origin.x + width < px(0.)
                || origin.y + height < px(0.)
                || origin.x > viewport.width
                || origin.y > viewport.height;
            (!hidden).then_some((index, node, origin, width))
        });
        let nodes = visible
            .map(|(index, node, origin, width)| {
                div()
                    .absolute()
                    .left(origin.x)
                    .top(origin.y)
                    .w(width)
                    .bg(colors.elevated_surface_background)
                    .border_1()
                    .border_color(colors.border)
                    .rounded(px(4. * zoom))
                    .text_size(px(12. * zoom))
                    .overflow_hidden()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            this.drag = Some(Drag::Node {
                                index,
                                last: event.position,
                            });
                            cx.stop_propagation();
                        }),
                    )
                    .child(
                        div()
                            .h(px(HEADER_HEIGHT * zoom))
                            .px(px(8. * zoom))
                            .flex()
                            .items_center()
                            .bg(colors.title_bar_background)
                            .child(node.name.clone()),
                    )
                    .children(node.columns.iter().map(|column| {
                        div()
                            .h(px(COLUMN_HEIGHT * zoom))
                            .px(px(8. * zoom))
                            .flex()
                            .items_center()
                            .text_color(colors.text_muted)
                            .child(column.clone())
                    }))
            })
            .collect::<Vec<_>>();

        div()
            .id("schema-map")
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(colors.editor_background)
            .text_color(colors.text)
            .on_scroll_wheel(cx.listener(Self::scroll))
            .on_mouse_move(cx.listener(Self::drag))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, _| {
                    this.drag = Some(Drag::Canvas {
                        last: event.position,
                    });
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.drag = None),
            )
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        for (from, to) in &segments {
                            let (from, to) = (bounds.origin + *from, bounds.origin + *to);
                            let bend = (from.x - to.x).abs() * 0.5;
                            let mut path = PathBuilder::stroke(px(1.5));
                            path.move_to(from);
                            path.cubic_bezier_to(
                                to,
                                point(from.x - bend, from.y),
                                point(to.x + bend, to.y),
                            );
                            if let Ok(path) = path.build() {
                                window.paint_path(path, edge_color);
                            }
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
            .children(nodes)
    }
}
