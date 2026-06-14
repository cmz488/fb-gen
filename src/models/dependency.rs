use crate::models::error::{FbGenError, FbGenResult};
use petgraph::algo;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The type of dependency relationship.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DependencyType {
    Public,
    Private,
    Interface,
}

/// A directed edge in the dependency graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyEdge {
    pub from: String,
    pub to: String,
    pub dep_type: DependencyType,
}

/// Wraps a petgraph `DiGraph` to model module-level dependencies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyGraph {
    graph: DiGraph<String, DependencyEdge>,
    node_map: HashMap<String, NodeIndex>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            node_map: HashMap::new(),
        }
    }

    /// Add a module node. Does nothing if the module already exists.
    pub fn add_module(&mut self, name: &str) -> NodeIndex {
        if let Some(&idx) = self.node_map.get(name) {
            return idx;
        }
        let idx = self.graph.add_node(name.to_string());
        self.node_map.insert(name.to_string(), idx);
        idx
    }

    /// Add a directed dependency edge between two modules.
    pub fn add_dependency(&mut self, edge: DependencyEdge) {
        let from_idx = self.add_module(&edge.from);
        let to_idx = self.add_module(&edge.to);
        self.graph.add_edge(from_idx, to_idx, edge);
    }

    /// Return the names and dependency types of all modules that `name` directly depends on.
    pub fn get_dependencies(&self, name: &str) -> Vec<(String, DependencyType)> {
        if let Some(&idx) = self.node_map.get(name) {
            self.graph
                .edges_directed(idx, petgraph::Direction::Outgoing)
                .filter_map(|edge| {
                    let target = self.graph.node_weight(edge.target())?;
                    Some((target.clone(), edge.weight().dep_type.clone()))
                })
                .collect()
        } else {
            vec![]
        }
    }

    /// Topological sort of all modules. Returns an error if a cycle exists.
    ///
    /// The result is in dependency-first order: modules with no dependencies
    /// come first, so they can be built before modules that depend on them.
    pub fn topological_order(&self) -> FbGenResult<Vec<String>> {
        algo::toposort(&self.graph, None)
            .map_err(|cycle_err| {
                let node_id = cycle_err.node_id();
                let name = self
                    .graph
                    .node_weight(node_id)
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string());
                FbGenError::CircularDependency(name)
            })
            .map(|indices| {
                // petgraph toposort puts source nodes first (A before B for edge A→B).
                // Our edges go from module→dependency, so we reverse to get
                // dependencies-first order for correct build sequencing.
                indices
                    .iter()
                    .rev()
                    .filter_map(|&ni| self.graph.node_weight(ni).cloned())
                    .collect()
            })
    }

    /// Check whether the dependency graph contains any cycles.
    pub fn has_cycles(&self) -> bool {
        algo::is_cyclic_directed(&self.graph)
    }

    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }
}

impl Default for DependencyGraph {
    fn default() -> Self {
        Self::new()
    }
}
