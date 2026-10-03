use super::movement::move_recorded_node;
use super::*;
use crate::schema_map_model::{MarkerGeometry, Path};
use gpui::{
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point,
    ScrollWheelEvent, canvas, point,
};
impl SchemaMapView {
    pub(super) fn fit(&mut self) {
        if let Some(scene) = &self.scene {
            match Camera::fit(
                scene.bounds(),
                self.viewport.width as f64,
                self.viewport.height as f64,
            ) {
                Ok(camera) => {
                    self.camera = camera;
                    self.fit_pending = false;
                }
                Err(error) => self.report(error),
            }
        }
    }
    fn local(&self, p: Point<Pixels>) -> MapPoint {
        MapPoint {
            x: f32::from(p.x) as f64 - self.canvas_origin.x,
            y: f32::from(p.y) as f64 - self.canvas_origin.y,
        }
    }
    fn pointer_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composing(window, cx) {
            return;
        }
        window.focus(&self.canvas_focus, cx);
        let local = self.local(event.position);
        let hit = self.scene.as_ref().and_then(|scene| {
            self.camera
                .to_world(local)
                .and_then(|p| scene.hit_test(scene.key(), p, (6. / self.camera.zoom).min(256.)))
        });
        self.select(hit);
        self.drag = match hit {
            Some(Selection::Node { identity, key }) if self.movement_allowed() => {
                Some(Drag::Node {
                    identity,
                    key,
                    last: local,
                    moved: false,
                })
            }
            Some(_) => None,
            None => Some(Drag::Pan { last: local }),
        };
        if event.click_count == 2 && matches!(hit, Some(Selection::Node { .. })) {
            self.drag = None;
            self.open_selected(cx);
        }
        cx.notify();
    }
    fn pointer_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let local = self.local(event.position);
        let Some(drag) = self.drag else {
            return;
        };
        match drag {
            Drag::Pan { last } => {
                if let Ok(camera) = self.camera.pan_by(MapPoint {
                    x: local.x - last.x,
                    y: local.y - last.y,
                }) {
                    self.camera = camera;
                }
                self.drag = Some(Drag::Pan { last: local });
            }
            Drag::Node {
                identity,
                key,
                last,
                moved,
            } => {
                if !self.movement_allowed() {
                    self.drag = None;
                    self.status =
                        "Map movement stopped because its capture or save authority changed".into();
                    cx.notify();
                    return;
                }
                let delta = MapPoint {
                    x: (local.x - last.x) / self.camera.zoom,
                    y: (local.y - last.y) / self.camera.zoom,
                };
                if delta.x == 0. && delta.y == 0. {
                    return;
                }
                let (Some(scene), Some(value)) = (&mut self.scene, &mut self.working) else {
                    self.drag = None;
                    return;
                };
                match move_recorded_node(scene, value, Selection::Node { identity, key }, delta) {
                    Ok(selection) => {
                        self.selection = Some(selection);
                        self.drag = Some(Drag::Node {
                            identity,
                            key: scene.key(),
                            last: local,
                            moved: true,
                        });
                        self.dirty = true;
                        self.status = "Node moved locally; position saves at drag end".into();
                    }
                    Err(error) => {
                        self.report(error);
                        self.drag = Some(Drag::Node {
                            identity,
                            key,
                            last: local,
                            moved,
                        });
                    }
                }
            }
        }
        cx.notify();
    }
    fn pointer_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        if let Drag::Node {
            identity,
            key,
            moved: true,
            ..
        } = drag
            && let Some(scene) = &self.scene
            && scene.key() == key
            && scene.node_selection(identity).is_some()
        {
            self.save_preferences(cx);
        }
        cx.notify();
    }
    fn wheel(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.composing(window, cx) || self.drag.is_some() {
            return;
        }
        let delta = event.delta.pixel_delta(window.line_height());
        let x = f32::from(delta.x) as f64;
        let y = f32::from(delta.y) as f64;
        let result = if event.modifiers.platform {
            self.camera.zoom_at(
                self.local(event.position),
                self.camera.zoom * (y / 400.).exp(),
            )
        } else {
            self.camera.pan_by(MapPoint { x, y })
        };
        if let Ok(camera) = result {
            self.camera = camera;
            cx.notify();
        }
        cx.stop_propagation();
    }
    pub(super) fn canvas(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let key = self.scene.as_ref().map(Scene::key);
        let camera = self.camera;
        let viewport = self.viewport;
        let nodes = self
            .scene
            .as_ref()
            .into_iter()
            .flat_map(|s| s.nodes())
            .filter_map(|node| {
                let origin = camera.to_screen(MapPoint {
                    x: node.bounds.x,
                    y: node.bounds.y,
                })?;
                let width = node.bounds.width * camera.zoom;
                let height = node.bounds.height * camera.zoom;

                if origin.x + width < 0.
                    || origin.y + height < 0.
                    || origin.x > viewport.width as f64
                    || origin.y > viewport.height as f64
                {
                    return None;
                }

                let selected = matches!(self.selection,Some(Selection::Node{
identity,..}
) if identity==node.identity);

                let first = (((-origin.y / camera.zoom) - crate::schema_map_model::HEADER_HEIGHT)
                    / crate::schema_map_model::ROW_HEIGHT)
                    .floor()
                    .max(0.) as usize;

                let last = (((viewport.height as f64 - origin.y) / camera.zoom
                    - crate::schema_map_model::HEADER_HEIGHT)
                    / crate::schema_map_model::ROW_HEIGHT)
                    .ceil()
                    .max(0.) as usize;

                let rows = node
                    .rows
                    .iter()
                    .enumerate()
                    .skip(first.min(node.rows.len()))
                    .take(last.saturating_sub(first) + 1)
                    .map(|(i, row)| {
                        div()
                            .absolute()
                            .top(px(((crate::schema_map_model::HEADER_HEIGHT
                                + i as f64 * crate::schema_map_model::ROW_HEIGHT)
                                * camera.zoom) as f32))
                            .left(px(8. * camera.zoom as f32))
                            .whitespace_nowrap()
                            .h(px(
                                (crate::schema_map_model::ROW_HEIGHT * camera.zoom) as f32
                            ))
                            .child(format!(
                                "{}{}{}",
                                row.label,
                                if row.foreign_key { "  FK" } else { "" },
                                if row.trigger_count > 0 {
                                    format!("  T{}", row.trigger_count)
                                } else {
                                    String::new()
                                }
                            ))
                    })
                    .collect::<Vec<_>>();

                Some(
                    div()
                        .absolute()
                        .left(px(origin.x as f32))
                        .top(px(origin.y as f32))
                        .w(px(width as f32))
                        .h(px(height as f32))
                        .bg(rgb(0))
                        .border_1()
                        .border_color(rgb(if selected { 0xffffff } else { 0x666666 }))
                        .text_size(px((12. * camera.zoom) as f32))
                        .overflow_hidden()
                        .child(
                            div()
                                .absolute()
                                .top(px((5. * camera.zoom) as f32))
                                .left(px((8. * camera.zoom) as f32))
                                .whitespace_nowrap()
                                .font_weight(gpui::FontWeight::BOLD)
                                .child(node.title.clone()),
                        )
                        .child(
                            div()
                                .absolute()
                                .top(px((24. * camera.zoom) as f32))
                                .left(px((8. * camera.zoom) as f32))
                                .text_size(px((10. * camera.zoom) as f32))
                                .whitespace_nowrap()
                                .child(format!(
                                    "{} columns{}{}{}",
                                    self.scene.as_ref().unwrap().snapshot().tables
                                        [node.table_index]
                                        .columns
                                        .len(),
                                    if node.junction { " | junction" } else { "" },
                                    if node.external { " | external" } else { "" },
                                    if node.trigger_count > 0 {
                                        format!(" | {} triggers", node.trigger_count)
                                    } else {
                                        String::new()
                                    }
                                )),
                        )
                        .children(rows),
                )
            })
            .collect::<Vec<_>>();

        let labels =
            self.scene
                .as_ref()
                .into_iter()
                .flat_map(|s| s.edges())
                .filter_map(|edge| {
                    let rect = edge.label_bounds();
                    let origin = camera.to_screen(MapPoint {
                        x: rect.x,
                        y: rect.y,
                    })?;
                    let width = rect.width * camera.zoom;
                    let height = rect.height * camera.zoom;
                    if origin.x + width < 0.
                        || origin.y + height < 0.
                        || origin.x > viewport.width as f64
                        || origin.y > viewport.height as f64
                    {
                        return None;
                    }
                    Some(
                        div()
                            .absolute()
                            .left(px(origin.x as f32))
                            .top(px(origin.y as f32))
                            .w(px(width as f32))
                            .h(px(height as f32))
                            .line_height(px(height as f32))
                            .text_center()
                            .whitespace_nowrap()
                            .text_size(px((crate::schema_map_model::EDGE_LABEL_FONT_SIZE
                                * camera.zoom) as f32))
                            .overflow_hidden()
                            .child(edge.label.clone()),
                    )
                })
                .collect::<Vec<_>>();
        let layout = cx.entity().downgrade();
        let paint = layout.clone();
        let drawing = canvas(
            move |bounds, _, cx| {
                let width = f32::from(bounds.size.width).max(1.).round() as u32;
                let height = f32::from(bounds.size.height).max(1.).round() as u32;
                let changed = layout.update(cx, |view, _| {
                    view.canvas_origin = MapPoint {
                        x: f32::from(bounds.origin.x) as f64,
                        y: f32::from(bounds.origin.y) as f64,
                    };
                    view.viewport.width != width || view.viewport.height != height || view.fit_pending
                }).unwrap_or(false);
                if changed {
                    // Keep this frame on its captured camera. Publish fitted
                    // geometry and invalidate only after prepaint has finished.
                    cx.defer(move |cx| {
                        layout.update(cx, |view, cx| {
                            view.viewport.width = width;
                            view.viewport.height = height;
                            if view.fit_pending { view.fit(); }
                            cx.notify();
                        }).ok();
                    });
                }
            },
            move |bounds, _, window, cx| {
                let Some(view) = paint.upgrade() else { return; };
                let view = view.read(cx);
                let Some(scene) = &view.scene else { return; };
                if Some(scene.key()) != key { return; }
                let to_screen = |p: MapPoint| {
                    let p = camera.to_screen(p).unwrap();
                    bounds.origin + point(px(p.x as f32), px(p.y as f32))
                };
                for edge in scene.edges() {
                    let selected = matches!(view.selection, Some(Selection::Edge { identity, .. }) if identity == edge.identity);
                    let color = rgb(if selected { 0xffffff } else { 0x888888 });
                    let mut path = PathBuilder::stroke(px(if selected { 2.5 } else { 1.5 }));
                    match &edge.path {
                        Path::Curve(p) => {
                            path.move_to(to_screen(p[0]));
                            path.cubic_bezier_to(to_screen(p[3]), to_screen(p[1]), to_screen(p[2]));
                        }
                        Path::Step(points) => {
                            path.move_to(to_screen(points[0]));
                            for p in &points[1..] { path.line_to(to_screen(*p)); }
                        }
                    }
                    if let Ok(path) = path.build() { window.paint_path(path, color); }
                    markers(&edge.source_geometry, &to_screen, window, color);
                    markers(&edge.target_geometry, &to_screen, window, color);
                }
            },
        ).absolute().size_full();
        div().id("schema-map-canvas").role(Role::Group)
            .aria_label("Schema map canvas. Drag background to pan; Command-scroll zooms at pointer; drag nodes to save positions. Keyboard arrows pan; Shift+Arrow moves the selected table 10 world units and saves; plus/minus zoom, F fits. The list below exposes exact tables and relationships.")
            .track_focus(&self.canvas_focus).tab_stop(true).tab_index(0)
            .relative().flex_1().min_h(px(140.)).overflow_hidden().bg(rgb(0))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::pointer_down))
            .on_mouse_move(cx.listener(Self::pointer_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::pointer_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::pointer_up))
            .on_scroll_wheel(cx.listener(Self::wheel))
            .child(drawing).children(labels).children(nodes)
    }
}

fn markers(
    marker: &MarkerGeometry,
    to_screen: &impl Fn(MapPoint) -> Point<Pixels>,
    window: &mut Window,
    color: gpui::Rgba,
) {
    let mut path = PathBuilder::stroke(px(1.5));
    for segment in marker.segments.iter().flatten() {
        path.move_to(to_screen(segment[0]));
        path.line_to(to_screen(segment[1]));
    }
    if let Some((center, radius)) = marker.circle {
        for i in 0..=16 {
            let a = i as f64 * std::f64::consts::TAU / 16.;
            let p = to_screen(MapPoint {
                x: center.x + radius * a.cos(),
                y: center.y + radius * a.sin(),
            });
            if i == 0 {
                path.move_to(p);
            } else {
                path.line_to(p);
            }
        }
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}
