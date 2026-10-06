# Graphs

`Whim\Graph` provides directed and undirected graphs, traversal, cycle checks,
topological sorting, and shortest paths. Graphs store nodes of type `TNode`
and edge weights of type `TWeight`. They compare nodes with Whim equality;
nodes do not need an ordering or a dictionary key type.

## Building a graph

Use `directed()` or `undirected()` to create an empty graph. `add_node()` and
`add_edge()` return a graph with the added node or edge. The original graph
keeps its nodes and edges. `add_edge()` adds either endpoint if it is missing:

```whim
use Whim\Graph;

$empty = Graph\directed::<string, int>();
$graph = Graph\add_edge::<string, int>($empty, 'A', 'B', 1);
$graph = Graph\add_edge::<string, int>($graph, 'A', 'C', 5);
$graph = Graph\add_edge::<string, int>($graph, 'B', 'C', 1);

assert!(Graph\nodes::<string, int>($empty) == vec[]);
assert!(Graph\nodes::<string, int>($graph) == vec['A', 'B', 'C']);
assert!(Graph\neighbors::<string, int>($graph, 'A') == vec['B', 'C']);
assert!(Graph\has_path::<string, int>($graph, 'A', 'C'));
assert!(!Graph\has_path::<string, int>($graph, 'C', 'A'));
assert!(Graph\bfs::<string, int>($graph, 'A') == vec['A', 'B', 'C']);
assert!(Graph\topological_sort::<string, int>($graph) == vec['A', 'B', 'C']);
assert!(Graph\shortest_path::<string>($graph, 'A', 'C') == vec['A', 'B', 'C']);
```

`DirectedGraph<TNode, TWeight>` and `UndirectedGraph<TNode, TWeight>` implement
the sealed `Graph<TNode, TWeight>` interface. Both classes are readonly.
The graph protects its structure; node values and weights may still hold
mutable objects.

Both classes provide these methods:

| Method | Result |
| --- | --- |
| `getNodes()` | All nodes in insertion order. |
| `getEdgesFrom(node)` | Outgoing `Edge<TNode, TWeight>` objects in insertion order. |
| `hasNode(node)` | Whether the node exists. |
| `hasEdge(from, to)` | Whether an edge connects the nodes. |
| `withNode(node)` | A graph containing the node. |
| `withEdge(from, edge)` | A graph containing the edge; both endpoints must already exist. |
| `hasCycle()` | Whether the graph contains a cycle. |

`Edge<TNode, TWeight>` holds a public readonly `to` node and a
`null|TWeight` weight. Its weight defaults to `null`.

You can also pass a node vector and a `vec<(TNode, Edge<TNode, TWeight>)>` to
either graph constructor. A constructor throws
`Whim\Unwind\InvalidArgumentException` if an edge refers to a missing node.
An undirected graph adds reverse edges with the same weights.

## Traversal and ordering

| Function | Result |
| --- | --- |
| `nodes(graph)` | The node vector. |
| `neighbors(graph, node)` | Direct neighbors in edge order. |
| `has_path(graph, from, to)` | Whether a path connects two existing nodes. |
| `has_cycle(graph)` | Whether a cycle exists. |
| `bfs(graph, start)` | Nodes reached in breadth-first order. |
| `dfs(graph, start)` | Nodes reached in depth-first order. |
| `topological_sort(directedGraph)` | A node order in which each edge's source precedes its destination, or `null` for a cycle. |

Traversals visit each reached node once and follow edges in insertion order.
They return an empty vector when the start node is missing. `has_path()` returns
`false` when either endpoint is missing; an existing node has a path to itself.
Topological sorting returns an empty vector for an empty graph.

## Shortest paths

`shortest_path()` accepts a graph with integer weights. `shortest_path_by()`
accepts any weight type and a callback that converts each weight to an integer
cost. Both return a nonempty vector from source to destination, or `null` if
an endpoint is missing or the destination is unreachable. An edge with a
`null` weight costs `1`; the conversion callback does not run for that edge.

```whim
use Whim\Graph;

$graph = Graph\directed::<string, string>();
$graph = Graph\add_edge::<string, string>($graph, 'A', 'B', 'long');
$graph = Graph\add_edge::<string, string>($graph, 'A', 'C', 'x');
$graph = Graph\add_edge::<string, string>($graph, 'C', 'B', 'x');

$path = Graph\shortest_path_by::<string, string>(
  $graph,
  'A',
  'B',
  fn(string $weight): int => length!($weight) as int,
);
assert!($path == vec['A', 'C', 'B']);
```

The search uses Dijkstra's algorithm for nonnegative costs and Bellman-Ford
when any cost is negative. It throws `NegativeCycleException` when a negative
cycle is reachable from the source and can reach the destination. A cycle
elsewhere does not make that path fail. A path cost outside the integer range
throws `Whim\Unwind\OverflowError`.
