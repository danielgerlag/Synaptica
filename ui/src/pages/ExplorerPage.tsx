import { useState } from 'react'
import { cn } from '@/lib/utils'
import { NodeList } from '@/components/explorer/NodeList'
import { EdgeList } from '@/components/explorer/EdgeList'
import { NodeForm } from '@/components/explorer/NodeForm'

const DEMO_NODES = [
  { id: 'a1b2c3d4', labels: ['Person'], properties: { name: 'Alice', age: 30, email: 'alice@example.com' } },
  { id: 'e5f6g7h8', labels: ['Person'], properties: { name: 'Bob', age: 25, email: 'bob@example.com' } },
  { id: 'i9j0k1l2', labels: ['Company'], properties: { name: 'Acme Corp', industry: 'Tech' } },
]

const DEMO_EDGES = [
  { id: 'x1y2z3w4', label: 'KNOWS', source: 'a1b2c3d4', target: 'e5f6g7h8', properties: { since: 2020, strength: 0.8 } },
  { id: 'q5r6s7t8', label: 'WORKS_AT', source: 'a1b2c3d4', target: 'i9j0k1l2', properties: { role: 'Engineer', since: 2019 } },
  { id: 'u9v0w1x2', label: 'WORKS_AT', source: 'e5f6g7h8', target: 'i9j0k1l2', properties: { role: 'Designer', since: 2021 } },
]

type Tab = 'nodes' | 'edges'

export function ExplorerPage() {
  const [tab, setTab] = useState<Tab>('nodes')
  const [showForm, setShowForm] = useState(false)

  const nodeLabels = Array.from(new Set(DEMO_NODES.flatMap((n) => n.labels)))
  const edgeLabels = Array.from(new Set(DEMO_EDGES.map((e) => e.label)))

  return (
    <div className="flex h-full flex-col gap-4">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">Data Explorer</h1>
        <button
          onClick={() => setShowForm(!showForm)}
          className="rounded bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90"
        >
          {tab === 'nodes' ? 'Create Node' : 'Create Edge'}
        </button>
      </div>

      <div className="flex gap-1 border-b border-border">
        {(['nodes', 'edges'] as const).map((t) => (
          <button
            key={t}
            onClick={() => {
              setTab(t)
              setShowForm(false)
            }}
            className={cn(
              'px-4 py-2 text-sm font-medium capitalize transition-colors',
              tab === t
                ? 'border-b-2 border-primary text-foreground'
                : 'text-muted-foreground hover:text-foreground'
            )}
          >
            {t}
          </button>
        ))}
      </div>

      {showForm && tab === 'nodes' && (
        <NodeForm
          onSave={(data) => {
            console.log('Create node:', data)
            setShowForm(false)
          }}
          onCancel={() => setShowForm(false)}
        />
      )}

      {showForm && tab === 'edges' && (
        <NodeForm
          initialLabels={[]}
          onSave={(data) => {
            console.log('Create edge:', data)
            setShowForm(false)
          }}
          onCancel={() => setShowForm(false)}
        />
      )}

      <div className="rounded-lg border border-border bg-card p-4">
        {tab === 'nodes' && (
          <NodeList nodes={DEMO_NODES} availableLabels={nodeLabels} />
        )}
        {tab === 'edges' && (
          <EdgeList edges={DEMO_EDGES} availableLabels={edgeLabels} />
        )}
      </div>
    </div>
  )
}
