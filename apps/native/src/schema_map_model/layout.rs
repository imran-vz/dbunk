use super::*;
const RANK_GAP: f64 = 140.;
const NODE_GAP: f64 = 40.;
const COMPONENT_GAP: f64 = 100.;
fn adjacency(
    nodes: &[Node],
    foreign_keys: &[SchemaMapForeignKey],
) -> Result<Vec<Vec<usize>>, &'static str> {
    let indices = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.identity, i))
        .collect::<BTreeMap<_, _>>();
    let mut edges = vec![Vec::new(); nodes.len()];
    for fk in foreign_keys {
        let source = *indices.get(&fk.source).ok_or("Missing map source node")?;
        let target = *indices.get(&fk.target).ok_or("Missing map target node")?;
        edges[source].push(target);
    }
    for row in &mut edges {
        row.sort_unstable();
        row.dedup();
    }
    Ok(edges)
}
fn finish(node: usize, edges: &[Vec<usize>], seen: &mut [bool], order: &mut Vec<usize>) {
    if seen[node] {
        return;
    }
    seen[node] = true;
    for &next in &edges[node] {
        finish(next, edges, seen, order);
    }
    order.push(node);
}
fn assign(node: usize, edges: &[Vec<usize>], group: usize, groups: &mut [usize]) {
    if groups[node] != usize::MAX {
        return;
    }
    groups[node] = group;
    for &next in &edges[node] {
        assign(next, edges, group, groups);
    }
}
/// Kosaraju over at most 512 nodes. SCC condensation avoids infinite rank
/// propagation for cycles/self-links; every traversal uses stable node order.
fn components(edges: &[Vec<usize>]) -> (Vec<usize>, Vec<Vec<usize>>) {
    let mut reverse = vec![Vec::new(); edges.len()];
    for (from, row) in edges.iter().enumerate() {
        for &to in row {
            reverse[to].push(from);
        }
    }
    let mut order = Vec::new();
    let mut seen = vec![false; edges.len()];
    for node in 0..edges.len() {
        finish(node, edges, &mut seen, &mut order);
    }
    let mut groups = vec![usize::MAX; edges.len()];
    let mut count = 0;
    for &node in order.iter().rev() {
        if groups[node] == usize::MAX {
            assign(node, &reverse, count, &mut groups);
            count += 1;
        }
    }
    let mut members = vec![Vec::new(); count];
    for (node, &group) in groups.iter().enumerate() {
        members[group].push(node);
    }
    (groups, members)
}
fn ranks(edges: &[Vec<usize>]) -> Vec<usize> {
    let (groups, members) = components(edges);
    let mut arcs = vec![BTreeSet::new(); members.len()];
    let mut indegree = vec![0; members.len()];
    for (from, row) in edges.iter().enumerate() {
        for &to in row {
            if groups[from] != groups[to] && arcs[groups[from]].insert(groups[to]) {
                indegree[groups[to]] += 1;
            }
        }
    }
    let mut ready = BTreeSet::new();
    for (g, degree) in indegree.iter().enumerate() {
        if *degree == 0 {
            ready.insert((members[g][0], g));
        }
    }
    let mut rank = vec![0; members.len()];
    while let Some((_, g)) = ready.pop_first() {
        for &next in &arcs[g] {
            rank[next] = rank[next].max(rank[g] + 1);
            indegree[next] -= 1;
            if indegree[next] == 0 {
                ready.insert((members[next][0], next));
            }
        }
    }
    groups.into_iter().map(|g| rank[g]).collect()
}
fn connected(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let mut undirected = edges.to_vec();
    for (from, row) in edges.iter().enumerate() {
        for &to in row {
            undirected[to].push(from);
        }
    }
    let mut seen = vec![false; edges.len()];
    let mut result = Vec::new();
    for node in 0..edges.len() {
        if seen[node] {
            continue;
        }
        let mut stack = vec![node];
        seen[node] = true;
        let mut group = Vec::new();
        while let Some(n) = stack.pop() {
            group.push(n);
            for &next in &undirected[n] {
                if !seen[next] {
                    seen[next] = true;
                    stack.push(next);
                }
            }
        }
        group.sort_unstable();
        result.push(group);
    }
    result
}
pub(super) fn place(nodes: &mut [Node], snapshot: &SchemaMapSnapshot) -> Result<(), &'static str> {
    if nodes.is_empty() {
        return Ok(());
    }
    let foreign_keys = &snapshot.foreign_keys;
    let edges = adjacency(nodes, foreign_keys)?;
    let indices = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.identity, i))
        .collect::<BTreeMap<_, _>>();
    let widths = foreign_keys
        .iter()
        .map(|fk| {
            label(|w| write_edge_label(w, snapshot, fk), false)
                .map(|l| l.advance as f64 + EDGE_LABEL_PADDING)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let ranks = ranks(&edges);
    let components = connected(&edges);
    let mut y = 40.;
    for component in components {
        let maximum = component.iter().map(|i| ranks[*i]).max().unwrap_or(0);
        let mut layers = vec![Vec::new(); maximum + 1];
        for &node in &component {
            layers[ranks[node]].push(node);
        }
        let mut gaps = vec![RANK_GAP; maximum + 1];
        for (fk, width) in foreign_keys.iter().zip(&widths) {
            let source = indices[&fk.source];
            let target = indices[&fk.target];
            if component.contains(&source) && ranks[source] < ranks[target] {
                gaps[ranks[source]] = gaps[ranks[source]].max(*width + 24.);
            }
        }
        let mut xs = vec![40.; maximum + 1];
        for rank in 1..xs.len() {
            xs[rank] = xs[rank - 1] + NODE_WIDTH + gaps[rank - 1];
        }
        // Four fixed barycentric passes improve crossings without unbounded
        // optimization. Stable node-index ties make reloads reproducible.
        for _ in 0..4 {
            let mut ordinal = vec![0.; nodes.len()];
            for layer in &layers {
                for (i, node) in layer.iter().enumerate() {
                    ordinal[*node] = i as f64;
                }
            }
            for (rank, layer) in layers.iter_mut().enumerate() {
                layer.sort_by(|a, b| {
                    let score = |node: usize| {
                        let mut count = 0;
                        let mut sum = 0.;
                        for (from, row) in edges.iter().enumerate() {
                            if ranks[from] < rank && row.contains(&node) {
                                sum += ordinal[from];
                                count += 1;
                            }
                        }
                        if count == 0 {
                            ordinal[node]
                        } else {
                            sum / f64::from(count)
                        }
                    };
                    score(*a).total_cmp(&score(*b)).then(a.cmp(b))
                });
            }
        }
        let heights = layers
            .iter()
            .map(|layer| {
                layer
                    .iter()
                    .map(|i| nodes[*i].bounds.height + NODE_GAP)
                    .sum::<f64>()
                    .max(NODE_GAP)
                    - NODE_GAP
            })
            .collect::<Vec<_>>();
        let height = heights.iter().copied().fold(0., f64::max);
        for (rank, layer) in layers.iter().enumerate() {
            let mut local = y + (height - heights[rank]) / 2.;
            for &node in layer {
                nodes[node].bounds.x = xs[rank];
                nodes[node].bounds.y = local;
                local += nodes[node].bounds.height + NODE_GAP;
            }
        }
        y += height + COMPONENT_GAP;
    }
    if nodes.iter().all(|n| n.bounds.valid()) {
        Ok(())
    } else {
        Err("Map layout exceeds finite coordinate bounds")
    }
}
pub(super) fn anchor(node: &Node, attnum: i16, right: bool) -> MapPoint {
    MapPoint {
        x: node.bounds.x + if right { node.bounds.width } else { 0. },
        y: node.bounds.y
            + node
                .rows
                .iter()
                .find(|r| r.attnum == attnum)
                .map_or(HEADER_HEIGHT / 2., |r| r.center_y),
    }
}
pub(super) fn edges(
    nodes: &[Node],
    snapshot: &SchemaMapSnapshot,
    routing: MapRouting,
) -> Result<Vec<Edge>, &'static str> {
    let indices = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.identity, i))
        .collect::<BTreeMap<_, _>>();
    let mut order = (0..snapshot.foreign_keys.len()).collect::<Vec<_>>();
    order.sort_by_key(|i| snapshot.foreign_keys[*i].constraint_oid);
    let mut result = Vec::with_capacity(order.len());
    for index in order {
        let fk = &snapshot.foreign_keys[index];
        let source = *indices.get(&fk.source).ok_or("Missing edge source")?;
        let target = *indices.get(&fk.target).ok_or("Missing edge target")?;
        let label = make_label(|w| write_edge_label(w, snapshot, fk))?;
        let geometry = route(nodes, source, target, fk, routing, edge_label_width(&label))?;
        let path = if routing == MapRouting::Curve {
            Path::Curve(geometry.points[..4].try_into().unwrap())
        } else {
            let mut points = Vec::with_capacity(6);
            points.extend_from_slice(&geometry.points[..geometry.count]);
            Path::Step(points)
        };
        let label_position = geometry.label;
        let source_marker = if fk.columns_unique {
            Marker::One
        } else {
            Marker::Many
        };
        let target_marker = if fk.columns_nullable {
            Marker::ZeroOrOne
        } else {
            Marker::One
        };
        let points = path.points();
        let source_geometry = marker(source_marker, points[0], points[1]);
        let target_geometry = marker(
            target_marker,
            points[points.len() - 1],
            points[points.len() - 2],
        );
        result.push(Edge {
            identity: EdgeIdentity {
                database_oid: fk.database_oid,
                constraint_oid: fk.constraint_oid,
            },
            foreign_key_index: index,
            source,
            target,
            path,
            label,
            label_position,
            source_marker,
            target_marker,
            source_geometry,
            target_geometry,
        });
    }
    Ok(result)
}
pub(super) fn bounds(nodes: &[Node], edges: &[Edge]) -> Result<Rect, &'static str> {
    let mut bounds = None;
    for node in nodes {
        if !node.bounds.valid() {
            return Err("Node position exceeds map bounds");
        }
        bounds = Some(bounds.map_or(node.bounds, |b: Rect| b.union(node.bounds)));
    }
    for edge in edges {
        let raw = edge.path.bounds();
        let path = Rect {
            x: raw.x - 24.,
            y: raw.y - 24.,
            width: raw.width + 48.,
            height: raw.height + 48.,
        };
        bounds = Some(bounds.map_or(path, |b| b.union(path)));
        let label = edge.label_bounds();
        if !label.valid() {
            return Err("Map label exceeds finite bounds");
        }
        bounds = Some(bounds.unwrap().union(label));
    }
    let bounds = bounds.unwrap_or(Rect {
        x: 0.,
        y: 0.,
        width: 1.,
        height: 1.,
    });
    if bounds.valid() {
        Ok(bounds)
    } else {
        Err("Map extent exceeds finite bounds")
    }
}

pub(super) fn marker(kind: Marker, tip: MapPoint, inside: MapPoint) -> MarkerGeometry {
    let dx = inside.x - tip.x;
    let dy = inside.y - tip.y;
    let length = dx.hypot(dy).max(1.);
    let ux = dx / length;
    let uy = dy / length;
    let point = |along: f64, across: f64| MapPoint {
        x: tip.x + ux * along - uy * across,
        y: tip.y + uy * along + ux * across,
    };
    match kind {
        Marker::One => MarkerGeometry {
            segments: [Some([point(8., -5.), point(8., 5.)]), None, None],
            circle: None,
        },
        Marker::ZeroOrOne => MarkerGeometry {
            segments: [Some([point(7., -5.), point(7., 5.)]), None, None],
            circle: Some((point(17., 0.), 4.)),
        },
        Marker::Many => MarkerGeometry {
            segments: [
                Some([tip, point(12., 0.)]),
                Some([point(0., -5.), point(12., 0.)]),
                Some([point(0., 5.), point(12., 0.)]),
            ],
            circle: None,
        },
    }
}

pub(super) struct Route {
    pub points: [MapPoint; 6],
    pub count: usize,
    pub label: MapPoint,
}
pub(super) fn route(
    nodes: &[Node],
    source_index: usize,
    target_index: usize,
    fk: &SchemaMapForeignKey,
    routing: MapRouting,
    label_width: f64,
) -> Result<Route, &'static str> {
    let source = &nodes[source_index];
    let target = &nodes[target_index];
    let pair = fk.columns.first().ok_or("Missing edge anchors")?;
    let from = anchor(source, pair.source, true);
    let to = anchor(target, pair.target, false);
    let lane = 32. + (fk.constraint_oid % 8) as f64 * 12.;
    let forward = from.x < to.x;
    let top = source.bounds.y.min(target.bounds.y) - lane;
    let mut points = [from; 6];
    let count;
    if routing == MapRouting::Curve {
        let bend = if forward {
            ((to.x - from.x) / 2.).max(40.)
        } else {
            lane + 60.
        };
        let same = source.identity == target.identity;
        points[..4].copy_from_slice(&[
            from,
            MapPoint {
                x: from.x + bend,
                y: if same { top } else { from.y },
            },
            MapPoint {
                x: to.x - bend,
                y: if same { top } else { to.y },
            },
            to,
        ]);
        count = 4;
    } else if forward {
        let x = (from.x + to.x) / 2.;
        points[..4].copy_from_slice(&[
            from,
            MapPoint { x, y: from.y },
            MapPoint { x, y: to.y },
            to,
        ]);
        count = 4;
    } else {
        points = [
            from,
            MapPoint {
                x: from.x + lane,
                y: from.y,
            },
            MapPoint {
                x: from.x + lane,
                y: top,
            },
            MapPoint {
                x: to.x - lane,
                y: top,
            },
            MapPoint {
                x: to.x - lane,
                y: to.y,
            },
            to,
        ];
        count = 6;
    }
    let mut label = MapPoint {
        // A forward caption stays in the source rank's outgoing gap even
        // when the FK skips ranks, avoiding intermediate table fills.
        x: if forward {
            from.x + 12. + label_width / 2.
        } else {
            (from.x + to.x) / 2.
        },
        y: if forward {
            (from.y + to.y) / 2. - 8.
        } else {
            top - 8.
        },
    };
    // Saved/dragged positions may close the outgoing gap. If obstructed,
    // place the caption above the highest horizontally overlapping node that
    // starts above its original bottom. Two bounded scans avoid iterative
    // collision solving during pointer movement and make rebuild/drag agree.
    let rect = caption_bounds(label, label_width);
    if nodes.iter().any(|n| n.bounds.intersects(rect)) {
        let top = nodes
            .iter()
            .filter(|n| {
                n.bounds.x <= rect.x + rect.width
                    && n.bounds.x + n.bounds.width >= rect.x
                    && n.bounds.y <= rect.y + rect.height
            })
            .map(|n| n.bounds.y)
            .reduce(f64::min)
            .expect("an obstructing node");
        label.y = top - 8. - (EDGE_LABEL_HEIGHT - EDGE_LABEL_BASELINE);
    }
    if !points[..count].iter().copied().all(camera::valid_point)
        || !caption_bounds(label, label_width).valid()
    {
        return Err("Edge route exceeds finite coordinate bounds");
    }
    let raw = Rect::from_points(&points[..count]).ok_or("Invalid edge points")?;
    if !(Rect {
        x: raw.x - 24.,
        y: raw.y - 24.,
        width: raw.width + 48.,
        height: raw.height + 48.,
    })
    .valid()
    {
        return Err("Edge marker extent exceeds map bounds");
    }
    Ok(Route {
        points,
        count,
        label,
    })
}
