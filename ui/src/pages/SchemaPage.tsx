import { useState, useEffect, useCallback } from 'react'
import { client, type SchemaLabel, type IndexInfo } from '@/lib/grpc-client'
import { useAppStore } from '@/lib/store'
import { LabelList } from '@/components/schema/LabelList'
import { IndexManager } from '@/components/schema/IndexManager'

export function SchemaPage() {
  const currentGraph = useAppStore((s) => s.currentGraph)
  const [nodeLabels, setNodeLabels] = useState<{ name: string; count: number; properties: string[] }[]>([])
  const [edgeLabels, setEdgeLabels] = useState<{ name: string; count: number; properties: string[] }[]>([])
  const [indexes, setIndexes] = useState<IndexInfo[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string>()
  const [seeding, setSeeding] = useState(false)

  const fetchData = useCallback(() => {
    setLoading(true)
    setError(undefined)
    Promise.all([client.getSchema(currentGraph), client.listIndexes(currentGraph)])
      .then(([schema, idxs]) => {
        setNodeLabels(schema.nodeSchemas.map((s: SchemaLabel) => ({
          name: s.label, count: s.count, properties: s.propertyKeys,
        })))
        setEdgeLabels(schema.edgeSchemas.map((s: SchemaLabel) => ({
          name: s.label, count: s.count, properties: s.propertyKeys,
        })))
        setIndexes(idxs)
      })
      .catch((err) => setError(err instanceof Error ? err.message : 'Failed to load schema'))
      .finally(() => setLoading(false))
  }, [currentGraph])

  useEffect(() => { fetchData() }, [fetchData])

  const handleSeed = async () => {
    setSeeding(true)
    try {
      await client.seedDemoData()
      fetchData()
    } finally {
      setSeeding(false)
    }
  }

  const isEmpty = nodeLabels.length === 0 && edgeLabels.length === 0

  return (
    <div className="flex h-full flex-col gap-4">
      <h1 className="text-2xl font-bold">Schema Browser</h1>

      {error && (
        <div className="rounded-lg border border-red-500/30 bg-red-500/10 p-3 text-sm text-red-400">
          {error}
        </div>
      )}

      {loading ? (
        <div className="flex items-center gap-2 py-12 justify-center text-muted-foreground">
          <span className="inline-block h-4 w-4 animate-spin rounded-full border-2 border-current border-t-transparent" />
          Loading schema…
        </div>
      ) : (
        <>
          {isEmpty && !error && (
            <div className="rounded-lg border border-border bg-card p-6 text-center">
              <p className="text-muted-foreground mb-3">No data in the database yet.</p>
              <button
                onClick={handleSeed}
                disabled={seeding}
                className="rounded bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
              >
                {seeding ? 'Seeding…' : 'Seed Demo Data'}
              </button>
            </div>
          )}

          <div className="grid gap-6 lg:grid-cols-2">
            <div className="rounded-lg border border-border bg-card p-4">
              <LabelList nodeLabels={nodeLabels} edgeLabels={edgeLabels} />
            </div>
            <div className="rounded-lg border border-border bg-card p-4">
              <IndexManager
                indexes={indexes}
                onCreateIndex={(idx) => {
                  client.createIndex(currentGraph, idx.name, idx.entityType, idx.propertyNames, idx.isUnique)
                    .then(() => fetchData())
                }}
                onDropIndex={(name) => {
                  client.dropIndex(currentGraph, name).then(() => fetchData())
                }}
              />
            </div>
          </div>
        </>
      )}
    </div>
  )
}
