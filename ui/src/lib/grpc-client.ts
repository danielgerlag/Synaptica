// gRPC-Web client for Synaptica — uses @protobuf-ts generated stubs.

import { GrpcWebFetchTransport } from '@protobuf-ts/grpcweb-transport'
import { SynapticaServiceClient } from '@/proto/client.client'
import type {
  GqlValue as ProtoGqlValue,
  GqlNode as ProtoGqlNode,
  GqlEdge as ProtoGqlEdge,
  QueryResponse,
  QueryStats as ProtoQueryStats,
} from '@/proto/client'

// ──────────────────── Public types ────────────────────

export interface QueryResult {
  columns: string[]
  rows: Record<string, unknown>[]
  stats?: QueryStats
  error?: string
}

export interface QueryStats {
  nodesCreated: number
  nodesDeleted: number
  edgesCreated: number
  edgesDeleted: number
  propertiesSet: number
  rowsReturned: number
  executionTimeMs: number
}

export interface LabelInfo {
  name: string
  count: number
}

export interface SchemaLabel {
  label: string
  propertyKeys: string[]
  count: number
}

export interface IndexInfo {
  name: string
  entityType: string
  propertyNames: string[]
  isUnique: boolean
}

export interface HealthInfo {
  status: string
  version: string
  uptimeSeconds: number
}

export interface ClusterStatus {
  nodeId: string
  role: string
  leaderId: string
  nodes: ClusterNodeInfo[]
  partitionCount: number
}

export interface ClusterNodeInfo {
  id: string
  address: string
  role: string
  isHealthy: boolean
}

export interface MetricsData {
  gauges: Record<string, number>
  counters: Record<string, number>
}

export interface GqlNode {
  id: string
  labels: string[]
  properties: Record<string, unknown>
}

export interface GqlEdge {
  id: string
  label: string
  sourceId: string
  targetId: string
  properties: Record<string, unknown>
}

// ──────────────────── Value conversion ────────────────────

function gqlValueToJs(v: ProtoGqlValue): unknown {
  switch (v.kind.oneofKind) {
    case 'nullValue': return null
    case 'boolValue': return v.kind.boolValue
    case 'integerValue': return Number(v.kind.integerValue)
    case 'floatValue': return v.kind.floatValue
    case 'stringValue': return v.kind.stringValue
    case 'bytesValue': return v.kind.bytesValue
    case 'listValue': return v.kind.listValue.items.map(gqlValueToJs)
    case 'mapValue': {
      const obj: Record<string, unknown> = {}
      for (const [k, val] of Object.entries(v.kind.mapValue.entries)) {
        obj[k] = gqlValueToJs(val)
      }
      return obj
    }
    case 'nodeValue': return protoNodeToJs(v.kind.nodeValue)
    case 'edgeValue': return protoEdgeToJs(v.kind.edgeValue)
    default: return null
  }
}

function protoPropsToJs(props: { [key: string]: ProtoGqlValue }): Record<string, unknown> {
  const result: Record<string, unknown> = {}
  for (const [k, v] of Object.entries(props)) {
    result[k] = gqlValueToJs(v)
  }
  return result
}

function protoNodeToJs(n: ProtoGqlNode): GqlNode {
  return { id: n.id, labels: [...n.labels], properties: protoPropsToJs(n.properties) }
}

function protoEdgeToJs(e: ProtoGqlEdge): GqlEdge {
  return { id: e.id, label: e.label, sourceId: e.sourceId, targetId: e.targetId, properties: protoPropsToJs(e.properties) }
}

function convertStats(s: ProtoQueryStats): QueryStats {
  return {
    nodesCreated: Number(s.nodesCreated),
    nodesDeleted: Number(s.nodesDeleted),
    edgesCreated: Number(s.edgesCreated),
    edgesDeleted: Number(s.edgesDeleted),
    propertiesSet: Number(s.propertiesSet),
    rowsReturned: Number(s.rowsReturned),
    executionTimeMs: Number(s.executionTimeMs),
  }
}

function convertQueryResponse(res: QueryResponse): QueryResult {
  const columns = [...res.columns]
  const rows = res.rows.map((row) => {
    const obj: Record<string, unknown> = {}
    row.values.forEach((v, i) => {
      obj[columns[i] ?? `col_${i}`] = gqlValueToJs(v)
    })
    return obj
  })
  return {
    columns,
    rows,
    stats: res.stats ? convertStats(res.stats) : undefined,
    error: res.error,
  }
}

// ──────────────────── Client ────────────────────

class SynapticaClient {
  private rpc: SynapticaServiceClient

  constructor(baseUrl: string = 'http://localhost:9090') {
    const transport = new GrpcWebFetchTransport({ baseUrl })
    this.rpc = new SynapticaServiceClient(transport)
  }

  setBaseUrl(url: string) {
    const transport = new GrpcWebFetchTransport({ baseUrl: url })
    this.rpc = new SynapticaServiceClient(transport)
  }

  async executeQuery(query: string, graphName: string = 'default'): Promise<QueryResult> {
    const { response } = await this.rpc.executeQuery({ query, graphName, parameters: {} })
    return convertQueryResponse(response)
  }

  async health(): Promise<HealthInfo> {
    const { response } = await this.rpc.health({})
    return {
      status: response.status,
      version: response.version,
      uptimeSeconds: Number(response.uptimeSeconds),
    }
  }

  async clusterStatus(): Promise<ClusterStatus> {
    const { response } = await this.rpc.clusterStatus({})
    return {
      nodeId: response.nodeId,
      role: response.role,
      leaderId: response.leaderId,
      nodes: response.nodes.map((n) => ({
        id: n.id,
        address: n.address,
        role: n.role,
        isHealthy: n.isHealthy,
      })),
      partitionCount: response.partitionCount,
    }
  }

  async listLabels(graphName: string = 'default'): Promise<{ nodeLabels: LabelInfo[]; edgeLabels: LabelInfo[] }> {
    const { response } = await this.rpc.listLabels({ graphName })
    return {
      nodeLabels: response.nodeLabels.map((l) => ({ name: l.name, count: Number(l.count) })),
      edgeLabels: response.edgeLabels.map((l) => ({ name: l.name, count: Number(l.count) })),
    }
  }

  async getSchema(graphName: string = 'default'): Promise<{ nodeSchemas: SchemaLabel[]; edgeSchemas: SchemaLabel[] }> {
    const { response } = await this.rpc.getSchema({ graphName })
    return {
      nodeSchemas: response.nodeSchemas.map((s) => ({
        label: s.label,
        propertyKeys: [...s.propertyKeys],
        count: Number(s.count),
      })),
      edgeSchemas: response.edgeSchemas.map((s) => ({
        label: s.label,
        propertyKeys: [...s.propertyKeys],
        count: Number(s.count),
      })),
    }
  }

  async listIndexes(graphName: string = 'default'): Promise<IndexInfo[]> {
    const { response } = await this.rpc.listIndexes({ graphName })
    return response.indexes.map((i) => ({
      name: i.name,
      entityType: i.entityType,
      propertyNames: [...i.propertyNames],
      isUnique: i.isUnique,
    }))
  }

  async createIndex(
    graphName: string, name: string, entityType: string,
    propertyNames: string[], isUnique: boolean,
  ): Promise<{ success: boolean; error?: string }> {
    const { response } = await this.rpc.createIndex({ graphName, name, entityType, propertyNames, isUnique })
    return { success: response.success, error: response.error }
  }

  async dropIndex(graphName: string, name: string): Promise<{ success: boolean; error?: string }> {
    const { response } = await this.rpc.dropIndex({ graphName, name })
    return { success: response.success, error: response.error }
  }

  async getMetrics(): Promise<MetricsData> {
    const { response } = await this.rpc.getMetrics({})
    const gauges: Record<string, number> = {}
    const counters: Record<string, number> = {}
    for (const [k, v] of Object.entries(response.gauges)) gauges[k] = v
    for (const [k, v] of Object.entries(response.counters)) counters[k] = Number(v)
    return { gauges, counters }
  }

  /** Run INSERT queries to populate the database with sample data. */
  async seedDemoData(): Promise<{ success: boolean; nodesCreated: number; edgesCreated: number }> {
    const nodeInserts = [
      "INSERT (:Person {name: 'Alice', age: 30, email: 'alice@example.com'})",
      "INSERT (:Person {name: 'Bob', age: 25, email: 'bob@example.com'})",
      "INSERT (:Person {name: 'Charlie', age: 35, city: 'Chicago'})",
      "INSERT (:Company {name: 'Acme Corp', industry: 'Tech'})",
      "INSERT (:Company {name: 'Globex', industry: 'Manufacturing'})",
      "INSERT (:City {name: 'New York', population: 8336817})",
      "INSERT (:City {name: 'San Francisco', population: 873965})",
      "INSERT (:City {name: 'Chicago', population: 2693976})",
    ]
    const edgeInserts = [
      "MATCH (a:Person {name: 'Alice'}), (b:Person {name: 'Bob'}) INSERT (a)-[:KNOWS {since: 2020}]->(b)",
      "MATCH (a:Person {name: 'Bob'}), (b:Person {name: 'Charlie'}) INSERT (a)-[:KNOWS {since: 2019}]->(b)",
      "MATCH (a:Person {name: 'Alice'}), (b:Person {name: 'Charlie'}) INSERT (a)-[:KNOWS {since: 2021}]->(b)",
      "MATCH (a:Person {name: 'Alice'}), (c:Company {name: 'Acme Corp'}) INSERT (a)-[:WORKS_AT {role: 'Engineer', since: 2019}]->(c)",
      "MATCH (a:Person {name: 'Bob'}), (c:Company {name: 'Acme Corp'}) INSERT (a)-[:WORKS_AT {role: 'Designer', since: 2021}]->(c)",
      "MATCH (a:Person {name: 'Charlie'}), (c:Company {name: 'Globex'}) INSERT (a)-[:WORKS_AT {role: 'Manager', since: 2018}]->(c)",
      "MATCH (a:Person {name: 'Alice'}), (c:City {name: 'New York'}) INSERT (a)-[:LIVES_IN]->(c)",
      "MATCH (a:Person {name: 'Bob'}), (c:City {name: 'San Francisco'}) INSERT (a)-[:LIVES_IN]->(c)",
      "MATCH (a:Person {name: 'Charlie'}), (c:City {name: 'Chicago'}) INSERT (a)-[:LIVES_IN]->(c)",
    ]

    let nodesCreated = 0
    let edgesCreated = 0
    for (const q of nodeInserts) {
      try {
        const r = await this.executeQuery(q)
        if (!r.error) nodesCreated += r.stats?.nodesCreated ?? 1
      } catch { /* continue */ }
    }
    for (const q of edgeInserts) {
      try {
        const r = await this.executeQuery(q)
        if (!r.error) edgesCreated += r.stats?.edgesCreated ?? 1
      } catch { /* continue */ }
    }
    return { success: true, nodesCreated, edgesCreated }
  }
}

export const client = new SynapticaClient()
