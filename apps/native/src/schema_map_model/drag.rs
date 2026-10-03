use super::*;
impl Scene {
    /// Changes admitted geometry without allocating. Call with a new layout
    /// revision, then persist the final positions at drag end. Invalid movement
    /// leaves the entire scene and its selection generation unchanged.
    pub fn move_node(
        &mut self,
        key: SceneKey,
        identity: SchemaMapIdentity,
        position: MapPoint,
    ) -> Result<(), &'static str> {
        if key.document_generation != self.key.document_generation
            || key.capture_generation != self.key.capture_generation
            || key.layout_revision <= self.key.layout_revision
        {
            return Err("Stale map drag generation");
        }
        let index = self
            .nodes
            .iter()
            .position(|n| n.identity == identity)
            .ok_or("Map drag target no longer exists")?;
        let old = self.nodes[index].bounds;
        let candidate = Rect {
            x: position.x,
            y: position.y,
            ..old
        };
        if !candidate.valid() {
            return Err("Invalid map drag coordinates");
        }
        self.nodes[index].bounds = candidate;
        // Caption placement depends on all node rectangles, including an
        // obstruction that just moved away. Dry-run every bounded route before
        // changing any edge; no strings or point buffers are allocated.
        for edge in &self.edges {
            if let Err(error) = layout::route(
                &self.nodes,
                edge.source,
                edge.target,
                &self.snapshot.foreign_keys[edge.foreign_key_index],
                self.prefs.routing,
                edge_label_width(&edge.label),
            ) {
                self.nodes[index].bounds = old;
                return Err(error);
            }
        }
        for edge in &mut self.edges {
            let route = layout::route(
                &self.nodes,
                edge.source,
                edge.target,
                &self.snapshot.foreign_keys[edge.foreign_key_index],
                self.prefs.routing,
                edge_label_width(&edge.label),
            )
            .expect("validated route");
            match &mut edge.path {
                Path::Curve(points) => points.copy_from_slice(&route.points[..4]),
                Path::Step(points) => {
                    points.clear();
                    points.extend_from_slice(&route.points[..route.count]);
                }
            }
            edge.label_position = route.label;
            edge.source_geometry =
                layout::marker(edge.source_marker, route.points[0], route.points[1]);
            edge.target_geometry = layout::marker(
                edge.target_marker,
                route.points[route.count - 1],
                route.points[route.count - 2],
            );
        }
        self.bounds = layout::bounds(&self.nodes, &self.edges)
            .expect("validated finite node and route bounds");
        self.key = key;
        Ok(())
    }
}
