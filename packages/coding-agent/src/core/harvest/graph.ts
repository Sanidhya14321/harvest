/**
 * Zero-Dependency Typed Knowledge Graph.
 *
 * Persisted at .harvest/graph.json.
 * Tracks directed relationships (depends_on, relates_to, supersedes, produced_by, contradicts).
 * Preserves dangling edges so relations become active when referenced nodes are created.
 * Supports BFS traversal and neighbor queries.
 */

import * as fs from "node:fs";
import * as path from "node:path";

export type EdgeRelation = "depends_on" | "relates_to" | "supersedes" | "produced_by" | "contradicts";

export interface GraphNode {
	readonly id: string;
	readonly role: string;
	readonly kind: "pattern" | "anti-pattern" | "skill" | "task";
	readonly title: string;
	readonly path: string;
	readonly metadata?: Record<string, unknown>;
}

export interface GraphEdge {
	readonly source: string;
	readonly target: string;
	readonly relation: EdgeRelation;
	readonly metadata?: Record<string, unknown>;
}

export interface GraphData {
	readonly version: number;
	readonly nodes: Record<string, GraphNode>;
	readonly edges: GraphEdge[];
}

export class HarvestKnowledgeGraph {
	readonly #graphFilePath: string;
	#data: GraphData;

	constructor(workspaceRoot: string = process.cwd()) {
		const harvestDir = path.join(path.resolve(workspaceRoot), ".harvest");
		this.#graphFilePath = path.join(harvestDir, "graph.json");
		this.#data = this.#load();
	}

	#load(): GraphData {
		try {
			if (fs.existsSync(this.#graphFilePath)) {
				const content = fs.readFileSync(this.#graphFilePath, "utf8");
				return JSON.parse(content);
			}
		} catch {}
		return {
			version: 1,
			nodes: {},
			edges: [],
		};
	}

	save(): void {
		try {
			const dir = path.dirname(this.#graphFilePath);
			fs.mkdirSync(dir, { recursive: true });
			fs.writeFileSync(this.#graphFilePath, JSON.stringify(this.#data, null, 2), "utf8");
		} catch {}
	}

	/** Add or update a node in the graph */
	addNode(node: GraphNode): void {
		this.#data = {
			...this.#data,
			nodes: {
				...this.#data.nodes,
				[node.id]: node,
			},
		};
		this.save();
	}

	getNode(id: string): GraphNode | undefined {
		return this.#data.nodes[id];
	}

	hasNode(id: string): boolean {
		return id in this.#data.nodes;
	}

	/**
	 * Add a directed edge.
	 * Preserves dangling edges even if target or source node is not yet registered.
	 */
	addEdge(source: string, target: string, relation: EdgeRelation, metadata?: Record<string, unknown>): void {
		// Prevent exact duplicate edges
		const exists = this.#data.edges.some(e => e.source === source && e.target === target && e.relation === relation);
		if (!exists) {
			this.#data = {
				...this.#data,
				edges: [...this.#data.edges, { source, target, relation, metadata }],
			};
			this.save();
		}
	}

	/** Get all outgoing edges from a node */
	getOutgoingEdges(nodeId: string): readonly GraphEdge[] {
		return this.#data.edges.filter(e => e.source === nodeId);
	}

	/** Get all incoming edges to a node */
	getIncomingEdges(nodeId: string): readonly GraphEdge[] {
		return this.#data.edges.filter(e => e.target === nodeId);
	}

	/**
	 * Repoint edges pointing to/from any node in oldNodeIds to newNodeId.
	 * Used during deterministic compaction to preserve graph connectivity.
	 */
	repointEdges(oldNodeIds: readonly string[], newNodeId: string): void {
		const oldSet = new Set(oldNodeIds);
		const newEdges: GraphEdge[] = [];

		for (const edge of this.#data.edges) {
			let source = edge.source;
			let target = edge.target;

			if (oldSet.has(source)) source = newNodeId;
			if (oldSet.has(target)) target = newNodeId;

			// Avoid self-loops after compaction
			if (source !== target) {
				const duplicate = newEdges.some(
					e => e.source === source && e.target === target && e.relation === edge.relation,
				);
				if (!duplicate) {
					newEdges.push({ ...edge, source, target });
				}
			}
		}

		this.#data = {
			...this.#data,
			edges: newEdges,
		};
		this.save();
	}

	/**
	 * BFS traversal to find all connected nodes within a max depth.
	 */
	bfs(startNodeId: string, maxDepth: number = 2): GraphNode[] {
		const visited = new Set<string>();
		const queue: Array<{ id: string; depth: number }> = [{ id: startNodeId, depth: 0 }];
		const result: GraphNode[] = [];

		visited.add(startNodeId);

		while (queue.length > 0) {
			const { id, depth } = queue.shift()!;
			const node = this.#data.nodes[id];
			if (node && id !== startNodeId) {
				result.push(node);
			}

			if (depth < maxDepth) {
				const edges = this.#data.edges.filter(e => e.source === id || e.target === id);
				for (const edge of edges) {
					const neighbor = edge.source === id ? edge.target : edge.source;
					if (!visited.has(neighbor)) {
						visited.add(neighbor);
						queue.push({ id: neighbor, depth: depth + 1 });
					}
				}
			}
		}

		return result;
	}

	get nodes(): Readonly<Record<string, GraphNode>> {
		return this.#data.nodes;
	}

	get edges(): readonly GraphEdge[] {
		return this.#data.edges;
	}
}
