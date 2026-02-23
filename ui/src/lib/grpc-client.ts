// gRPC-Web client for Synaptica
// This will be fully wired once proto generation is set up.
// For now, provides a fetch-based client that speaks gRPC-Web protocol.

const DEFAULT_URL = 'http://localhost:9090'

export interface QueryResult {
  columns: string[]
  rows: Record<string, unknown>[]
  stats?: {
    nodesCreated: number
    nodesDeleted: number
    edgesCreated: number
    edgesDeleted: number
    propertiesSet: number
    rowsReturned: number
    executionTimeMs: number
  }
  error?: string
}

export interface LabelInfo {
  name: string
  count: number
}

export interface HealthInfo {
  status: string
  version: string
  uptimeSeconds: number
}

// Simple JSON-over-HTTP client that wraps the gRPC calls
// In production, this would use proper gRPC-Web protobuf encoding
// For development, we'll use a REST-like wrapper
class SynapticaClient {
  private baseUrl: string

  constructor(baseUrl: string = DEFAULT_URL) {
    this.baseUrl = baseUrl
  }

  setBaseUrl(url: string) {
    this.baseUrl = url
  }

  async executeQuery(query: string, graphName: string = 'default'): Promise<QueryResult> {
    // Mock response for UI development
    console.log(`Executing query on ${this.baseUrl}: ${query} (graph: ${graphName})`)
    await new Promise((r) => setTimeout(r, 80 + Math.random() * 120))
    return {
      columns: ['name', 'age', 'city'],
      rows: [
        { name: 'Alice', age: 30, city: 'New York' },
        { name: 'Bob', age: 25, city: 'Los Angeles' },
        { name: 'Charlie', age: 35, city: 'Chicago' },
      ],
      stats: {
        nodesCreated: 0,
        nodesDeleted: 0,
        edgesCreated: 0,
        edgesDeleted: 0,
        propertiesSet: 0,
        rowsReturned: 3,
        executionTimeMs: 12,
      },
    }
  }

  async health(): Promise<HealthInfo> {
    return { status: 'unknown', version: '0.1.0', uptimeSeconds: 0 }
  }

  async listLabels(graphName: string = 'default'): Promise<{ nodeLabels: LabelInfo[]; edgeLabels: LabelInfo[] }> {
    return { nodeLabels: [], edgeLabels: [] }
  }
}

export const client = new SynapticaClient()
