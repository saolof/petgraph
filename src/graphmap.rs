//! `GraphMap<N, E>` is an undirected graph where node values are mapping keys.

use std::cmp::Ordering;
use std::hash::{self, Hash};
use std::iter::Cloned;
use std::slice::{
    Iter,
};
use std::fmt;
use std::ops::{Index, IndexMut, Deref};
use std::iter::FromIterator;
use std::marker::PhantomData;
use ordermap::OrderMap;
use ordermap::Iter as OrderMapIter;
use ordermap::Keys;

use {
    EdgeType,
    Directed,
    Undirected,
    EdgeDirection,
    Incoming,
    Outgoing,
};

use IntoWeightedEdge;
use visit::IntoNodeIdentifiers;
use graph::Graph;

/// A `GraphMap` with undirected edges.
///
/// For example, an edge between *1* and *2* is equivalent to an edge between
/// *2* and *1*.
pub type UnGraphMap<N, E> = GraphMap<N, E, Undirected>;
/// A `GraphMap` with directed edges.
///
/// For example, an edge from *1* to *2* is distinct from an edge from *2* to
/// *1*.
pub type DiGraphMap<N, E> = GraphMap<N, E, Directed>;

/// `GraphMap<N, E, Ty>` is a graph datastructure using an associative array
/// of its node weights `N`.
///
/// It uses an combined adjacency list and sparse adjacency matrix
/// representation, using **O(|V| + |E|)** space, and allows testing for edge
/// existance in constant time.
///
/// The edge type `Ty` can be `Directed` or `Undirected`.
///
/// You can use the type aliases `UnGraphMap` and `DiGraphMap` for convenience.
///
/// The node type `N` must implement `Copy` and will be used as node identifier, duplicated
/// into several places in the data structure.
/// It must be suitable as a hash table key (implementing `Eq + Hash`).
/// The node type must also implement `Ord` so that the implementation can
/// order the pair (`a`, `b`) for an edge connecting any two nodes `a` and `b`.
///
/// `GraphMap` does not allow parallel edges, but self loops are allowed.
#[derive(Clone)]
pub struct GraphMap<N, E, Ty> {
    nodes: OrderMap<N, Vec<(N, EdgeDirection)>>,
    edges: OrderMap<(N, N), E>,
    ty: PhantomData<Ty>,
}

impl<N: Eq + Hash + fmt::Debug, E: fmt::Debug, Ty: EdgeType> fmt::Debug for GraphMap<N, E, Ty> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.nodes.fmt(f)
    }
}

/// A trait group for `GraphMap`'s node identifier.
pub trait NodeTrait : Copy + Ord + Hash {}
impl<N> NodeTrait for N where N: Copy + Ord + Hash {}

impl<N, E, Ty> GraphMap<N, E, Ty>
    where N: NodeTrait,
          Ty: EdgeType,
{
    /// Create a new `GraphMap`
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new `GraphMap` with estimated capacity.
    pub fn with_capacity(nodes: usize, edges: usize) -> Self {
        GraphMap {
            nodes: OrderMap::with_capacity(nodes),
            edges: OrderMap::with_capacity(edges),
            ty: PhantomData,
        }
    }

    /// Return the current node and edge capacity of the graph.
    pub fn capacity(&self) -> (usize, usize) {
        (self.nodes.capacity(), self.edges.capacity())
    }

    /// Use their natual order to map the node pair (a, b) to a canonical edge id.
    #[inline]
    fn edge_key(a: N, b: N) -> (N, N) {
        if Ty::is_directed() {
            (a, b)
        } else {
            if a <= b { (a, b) } else { (b, a) }
        }
    }

    /// Whether the graph has directed edges.
    pub fn is_directed(&self) -> bool {
        Ty::is_directed()
    }

    /// Create a new `GraphMap` from an iterable of edges.
    ///
    /// Node values are taken directly from the list.
    /// Edge weights `E` may either be specified in the list,
    /// or they are filled with default values.
    ///
    /// Nodes are inserted automatically to match the edges.
    ///
    /// ```
    /// use petgraph::graphmap::UnGraphMap;
    ///
    /// // Create a new undirected GraphMap.
    /// // Use a type hint to have `()` be the edge weight type.
    /// let gr = UnGraphMap::<_, ()>::from_edges(&[
    ///     (0, 1), (0, 2), (0, 3),
    ///     (1, 2), (1, 3),
    ///     (2, 3),
    /// ]);
    /// ```
    pub fn from_edges<I>(iterable: I) -> Self
        where I: IntoIterator,
              I::Item: IntoWeightedEdge<E, NodeId=N>
    {
        Self::from_iter(iterable)
    }

    /// Return the number of nodes in the graph.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Return the number of edges in the graph.
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Remove all nodes and edges
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.edges.clear();
    }

    /// Add node `n` to the graph.
    pub fn add_node(&mut self, n: N) -> N {
        self.nodes.entry(n).or_insert(Vec::new());
        n
    }

    /// Return `true` if node `n` was removed.
    pub fn remove_node(&mut self, n: N) -> bool {
        let links = match self.nodes.swap_remove(&n) {
            None => return false,
            Some(sus) => sus,
        };
        for (succ, _) in links.into_iter() {
            // remove all successor links
            self.remove_single_edge(&succ, &n, Incoming);
            // Remove all edge values
            self.edges.swap_remove(&Self::edge_key(n, succ));
        }
        true
    }

    /// Return `true` if the node is contained in the graph.
    pub fn contains_node(&self, n: N) -> bool {
        self.nodes.contains_key(&n)
    }

    /// Add an edge connecting `a` and `b` to the graph, with associated
    /// data `weight`.
    ///
    /// Inserts nodes `a` and/or `b` if they aren't already part of the graph.
    ///
    /// Return `None` if the edge did not previously exist, otherwise,
    /// the associated data is updated and the old value is returned
    /// as `Some(old_weight)`.
    ///
    /// ```
    /// // Create a GraphMap with directed edges, and add one edge to it
    /// use petgraph::graphmap::DiGraphMap;
    ///
    /// let mut g = DiGraphMap::new();
    /// g.add_edge("x", "y", -1);
    /// assert_eq!(g.node_count(), 2);
    /// assert_eq!(g.edge_count(), 1);
    /// assert!(g.contains_edge("x", "y"));
    /// assert!(!g.contains_edge("y", "x"));
    /// ```
    pub fn add_edge(&mut self, a: N, b: N, weight: E) -> Option<E> {
        if let old @ Some(_) = self.edges.insert(Self::edge_key(a, b), weight) {
            old
        } else {
            // insert in the adjacency list if it's a new edge
            self.nodes.entry(a)
                      .or_insert_with(|| Vec::with_capacity(1))
                      .push((b, Outgoing));
            if a != b {
                // self loops don't have the Incoming entry
                self.nodes.entry(b)
                          .or_insert_with(|| Vec::with_capacity(1))
                          .push((a, Incoming));
            }
            None
        }
    }

    /// Remove edge relation from a to b
    ///
    /// Return `true` if it did exist.
    fn remove_single_edge(&mut self, a: &N, b: &N, dir: EdgeDirection) -> bool {
        match self.nodes.get_mut(a) {
            None => false,
            Some(sus) => {
                if Ty::is_directed() {
                    match sus.iter().position(|elt| elt == &(*b, dir)) {
                        Some(index) => { sus.swap_remove(index); true }
                        None => false,
                    }
                } else {
                    match sus.iter().position(|elt| &elt.0 == b) {
                        Some(index) => { sus.swap_remove(index); true }
                        None => false,
                    }
                }
            }
        }
    }

    /// Remove edge from `a` to `b` from the graph and return the edge weight.
    ///
    /// Return `None` if the edge didn't exist.
    ///
    /// ```
    /// // Create a GraphMap with undirected edges, and add and remove an edge.
    /// use petgraph::graphmap::UnGraphMap;
    ///
    /// let mut g = UnGraphMap::new();
    /// g.add_edge("x", "y", -1);
    ///
    /// let edge_data = g.remove_edge("y", "x");
    /// assert_eq!(edge_data, Some(-1));
    /// assert_eq!(g.edge_count(), 0);
    /// ```
    pub fn remove_edge(&mut self, a: N, b: N) -> Option<E> {
        let exist1 = self.remove_single_edge(&a, &b, Outgoing);
        let exist2 = if a != b { self.remove_single_edge(&b, &a, Incoming) } else { exist1 };
        let weight = self.edges.remove(&Self::edge_key(a, b));
        debug_assert!(exist1 == exist2 && exist1 == weight.is_some());
        weight
    }

    /// Return `true` if the edge connecting `a` with `b` is contained in the graph.
    pub fn contains_edge(&self, a: N, b: N) -> bool {
        self.edges.contains_key(&Self::edge_key(a, b))
    }

    /// Return an iterator over the nodes of the graph.
    ///
    /// Iterator element type is `N`.
    pub fn nodes(&self) -> Nodes<N> {
        Nodes{iter: self.nodes.keys().cloned()}
    }

    /// Return an iterator over the nodes that are connected with `from` by edges.
    ///
    /// If the node `from` does not exist in the graph, return an empty iterator.
    ///
    /// Iterator element type is `N`.
    pub fn neighbors(&self, from: N) -> Neighbors<N, Ty> {
        Neighbors {
            iter: match self.nodes.get(&from) {
                Some(neigh) => neigh.iter(),
                None => [].iter(),
            },
            ty: self.ty,
        }
    }

    /// Return an iterator of all neighbors that have an edge between `a`
    /// and themselves, in the specified direction.
    ///
    /// If the graph's edges are undirected, this is equivalent to *.neighbors(a)*.
    ///
    /// If the node `a` does not exist in the graph, return an empty iterator.
    ///
    /// Iterator element type is `N`.
    pub fn neighbors_directed(&self, a: N, dir: EdgeDirection)
        -> NeighborsDirected<N, Ty>
    {
        NeighborsDirected {
            iter: match self.nodes.get(&a) {
                Some(neigh) => neigh.iter(),
                None => [].iter(),
            },
            dir: dir,
            ty: self.ty,
        }
    }

    /// Return an iterator over the nodes that are connected with `from` by edges,
    /// paired with the edge weight.
    ///
    /// If the node `from` does not exist in the graph, return an empty iterator.
    ///
    /// Iterator element type is `(N, &E)`.
    pub fn edges(&self, from: N) -> Edges<N, E, Ty> {
        Edges {
            from: from,
            iter: self.neighbors(from),
            edges: &self.edges,
        }
    }

    /// Return a reference to the edge weight connecting `a` with `b`, or
    /// `None` if the edge does not exist in the graph.
    pub fn edge_weight(&self, a: N, b: N) -> Option<&E> {
        self.edges.get(&Self::edge_key(a, b))
    }

    /// Return a mutable reference to the edge weight connecting `a` with `b`, or
    /// `None` if the edge does not exist in the graph.
    pub fn edge_weight_mut(&mut self, a: N, b: N) -> Option<&mut E> {
        self.edges.get_mut(&Self::edge_key(a, b))
    }

    /// Return an iterator over all edges of the graph with their weight in arbitrary order.
    ///
    /// Iterator element type is `(N, N, &E)`
    pub fn all_edges(&self) -> AllEdges<N, E, Ty> {
        AllEdges {
            inner: self.edges.iter(),
            ty: self.ty,
        }
    }

    /// Return a `Graph` that corresponds to this `GraphMap`.
    ///
    /// Note: node and edge indices in the `Graph` have nothing in common
    /// with the `GraphMap`s node weights `N`. The node weights `N` are
    /// used as node weights in the resulting `Graph`, too.
    pub fn into_graph<Ix>(self) -> Graph<N, E, Ty, Ix>
        where Ix: ::graph::IndexType,
    {
        // assuming two successive iterations of the same hashmap produce the same order
        let mut gr = Graph::with_capacity(self.node_count(), self.edge_count());
        // map N -> NodeIndex
        let mut node_map = OrderMap::with_capacity(self.node_count());
        for (&node, _) in self.nodes.iter() {
            node_map.insert(node, gr.add_node(node));
        }
        for ((a, b), edge_weight) in self.edges.into_iter() {
            let ai = node_map[&a];
            let bi = node_map[&b];
            gr.add_edge(ai, bi, edge_weight);
        }
        gr
    }
}

/// Create a new `GraphMap` from an iterable of edges.
impl<N, E, Ty, Item> FromIterator<Item> for GraphMap<N, E, Ty>
    where Item: IntoWeightedEdge<E, NodeId=N>,
          N: NodeTrait,
          Ty: EdgeType,
{
    fn from_iter<I>(iterable: I) -> Self
        where I: IntoIterator<Item=Item>,
    {
        let iter = iterable.into_iter();
        let (low, _) = iter.size_hint();
        let mut g = Self::with_capacity(0, low);
        g.extend(iter);
        g
    }
}

/// Extend the graph from an iterable of edges.
///
/// Nodes are inserted automatically to match the edges.
impl<N, E, Ty, Item> Extend<Item> for GraphMap<N, E, Ty>
    where Item: IntoWeightedEdge<E, NodeId=N>,
          N: NodeTrait,
          Ty: EdgeType,
{
    fn extend<I>(&mut self, iterable: I)
        where I: IntoIterator<Item=Item>,
    {
        let iter = iterable.into_iter();
        let (low, _) = iter.size_hint();
        self.edges.reserve(low);

        for elt in iter {
            let (source, target, weight) = elt.into_weighted_edge();
            self.add_edge(source, target, weight);
        }
    }
}

macro_rules! iterator_wrap {
    ($name: ident <$($typarm:tt),*> where { $($bounds: tt)* }
     item: $item: ty,
     iter: $iter: ty,
     ) => (
        pub struct $name <$($typarm),*> where $($bounds)* {
            iter: $iter,
        }
        impl<$($typarm),*> Iterator for $name <$($typarm),*>
            where $($bounds)*
        {
            type Item = $item;
            #[inline]
            fn next(&mut self) -> Option<Self::Item> {
                self.iter.next()
            }

            #[inline]
            fn size_hint(&self) -> (usize, Option<usize>) {
                self.iter.size_hint()
            }
        }
    );
}

iterator_wrap! {
    Nodes <'a, N> where { N: 'a + NodeTrait }
    item: N,
    iter: Cloned<Keys<'a, N, Vec<(N, EdgeDirection)>>>,
}

pub struct Neighbors<'a, N, Ty = Undirected>
    where N: 'a,
          Ty: EdgeType,
{
    iter: Iter<'a, (N, EdgeDirection)>,
    ty: PhantomData<Ty>,
}

impl<'a, N, Ty> Iterator for Neighbors<'a, N, Ty>
    where N: NodeTrait,
          Ty: EdgeType
{
    type Item = N;
    fn next(&mut self) -> Option<N> {
        if Ty::is_directed() {
            (&mut self.iter)
                .filter_map(|&(n, dir)| if dir == Outgoing {
                    Some(n)
                } else { None })
                .next()
        } else {
            self.iter.next().map(|&(n, _)| n)
        }
    }
}

pub struct NeighborsDirected<'a, N, Ty>
    where N: 'a,
          Ty: EdgeType,
{
    iter: Iter<'a, (N, EdgeDirection)>,
    dir: EdgeDirection,
    ty: PhantomData<Ty>,
}

impl<'a, N, Ty> Iterator for NeighborsDirected<'a, N, Ty>
    where N: NodeTrait,
          Ty: EdgeType
{
    type Item = N;
    fn next(&mut self) -> Option<N> {
        if Ty::is_directed() {
            let self_dir = self.dir;
            (&mut self.iter)
                .filter_map(move |&(n, dir)| if dir == self_dir {
                    Some(n)
                } else { None })
                .next()
        } else {
            self.iter.next().map(|&(n, _)| n)
        }
    }
}

pub struct Edges<'a, N, E: 'a, Ty>
    where N: 'a + NodeTrait,
          Ty: EdgeType
{
    from: N,
    edges: &'a OrderMap<(N, N), E>,
    iter: Neighbors<'a, N, Ty>,
}

impl<'a, N, E, Ty> Iterator for Edges<'a, N, E, Ty>
    where N: 'a + NodeTrait, E: 'a,
          Ty: EdgeType,
{
    type Item = (N, &'a E);
    fn next(&mut self) -> Option<(N, &'a E)>
    {
        match self.iter.next() {
            None => None,
            Some(b) => {
                let a = self.from;
                match self.edges.get(&GraphMap::<N, E, Ty>::edge_key(a, b)) {
                    None => unreachable!(),
                    Some(edge) => {
                        Some((b, edge))
                    }
                }
            }
        }
    }
}

pub struct AllEdges<'a, N, E: 'a, Ty> where N: 'a + NodeTrait {
    inner: OrderMapIter<'a, (N, N), E>,
    ty: PhantomData<Ty>,
}

impl<'a, N, E, Ty> Iterator for AllEdges<'a, N, E, Ty>
    where N: 'a + NodeTrait, E: 'a,
          Ty: EdgeType,
{
    type Item = (N, N, &'a E);
    fn next(&mut self) -> Option<Self::Item>
    {
        match self.inner.next() {
            None => None,
            Some((&(a, b), v)) => Some((a, b, v))
        }
    }
}

/// Index `GraphMap` by node pairs to access edge weights.
impl<N, E, Ty> Index<(N, N)> for GraphMap<N, E, Ty>
    where N: NodeTrait,
          Ty: EdgeType,
{
    type Output = E;
    fn index(&self, index: (N, N)) -> &E
    {
        let index = Self::edge_key(index.0, index.1);
        self.edge_weight(index.0, index.1).expect("GraphMap::index: no such edge")
    }
}

/// Index `GraphMap` by node pairs to access edge weights.
impl<N, E, Ty> IndexMut<(N, N)> for GraphMap<N, E, Ty>
    where N: NodeTrait,
          Ty: EdgeType,
{
    fn index_mut(&mut self, index: (N, N)) -> &mut E {
        let index = Self::edge_key(index.0, index.1);
        self.edge_weight_mut(index.0, index.1).expect("GraphMap::index: no such edge")
    }
}

/// Create a new empty `GraphMap`.
impl<N, E, Ty> Default for GraphMap<N, E, Ty>
    where N: NodeTrait,
          Ty: EdgeType,
{
    fn default() -> Self { GraphMap::with_capacity(0, 0) }
}

/// A reference that is hashed and compared by its pointer value.
///
/// `Ptr` is used for certain configurations of `GraphMap`,
/// in particular in the combination where the node type for
/// `GraphMap` is something of type for example `Ptr(&Cell<T>)`,
/// with the `Cell<T>` being `TypedArena` allocated.
pub struct Ptr<'b, T: 'b>(pub &'b T);

impl<'b, T> Copy for Ptr<'b, T> {}
impl<'b, T> Clone for Ptr<'b, T>
{
    fn clone(&self) -> Self { *self }
}


fn ptr_eq<T>(a: *const T, b: *const T) -> bool {
    a == b
}

impl<'b, T> PartialEq for Ptr<'b, T>
{
    /// Ptr compares by pointer equality, i.e if they point to the same value
    fn eq(&self, other: &Ptr<'b, T>) -> bool {
        ptr_eq(self.0, other.0)
    }
}

impl<'b, T> PartialOrd for Ptr<'b, T>
{
    fn partial_cmp(&self, other: &Ptr<'b, T>) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<'b, T> Ord for Ptr<'b, T>
{
    /// Ptr is ordered by pointer value, i.e. an arbitrary but stable and total order.
    fn cmp(&self, other: &Ptr<'b, T>) -> Ordering {
        let a = self.0 as *const _;
        let b = other.0 as *const _;
        a.cmp(&b)
    }
}

impl<'b, T> Deref for Ptr<'b, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0
    }
}

impl<'b, T> Eq for Ptr<'b, T> {}

impl<'b, T> Hash for Ptr<'b, T>
{
    fn hash<H: hash::Hasher>(&self, st: &mut H)
    {
        let ptr = (self.0) as *const T;
        ptr.hash(st)
    }
}

impl<'b, T: fmt::Debug> fmt::Debug for Ptr<'b, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<'a, N, E: 'a, Ty> IntoNodeIdentifiers for &'a GraphMap<N, E, Ty>
    where N: NodeTrait,
          Ty: EdgeType,
{
    type NodeIdentifiers = NodeIdentifiers<'a, N, E, Ty>;

    fn node_identifiers(self) -> Self::NodeIdentifiers {
        NodeIdentifiers {
            iter: self.nodes.iter(),
            ty: self.ty,
            edge_ty: PhantomData,
        }
    }

    fn node_count(&self) -> usize {
        (*self).node_count()
    }
}

pub struct NodeIdentifiers<'a, N, E: 'a, Ty> where N: 'a + NodeTrait {
    iter: OrderMapIter<'a, N, Vec<(N, EdgeDirection)>>,
    ty: PhantomData<Ty>,
    edge_ty: PhantomData<E>,
}

impl<'a, N, E, Ty> Iterator for NodeIdentifiers<'a, N, E, Ty>
    where N: 'a + NodeTrait, E: 'a,
          Ty: EdgeType,
{
    type Item = N;
    fn next(&mut self) -> Option<Self::Item>
    {
        self.iter.next().map(|(&n, _)| n)
    }
}
