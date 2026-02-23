import { LabelList } from '@/components/schema/LabelList'
import { IndexManager } from '@/components/schema/IndexManager'

const DEMO_LABELS = {
  nodeLabels: [
    { name: 'Person', count: 150, properties: ['name', 'age', 'email'] },
    { name: 'Company', count: 45, properties: ['name', 'industry', 'founded'] },
    { name: 'City', count: 30, properties: ['name', 'population', 'country'] },
  ],
  edgeLabels: [
    { name: 'KNOWS', count: 320, properties: ['since', 'strength'] },
    { name: 'WORKS_AT', count: 180, properties: ['role', 'since'] },
    { name: 'LIVES_IN', count: 150, properties: [] },
  ],
}

const DEMO_INDEXES = [
  { name: 'idx_person_name', entityType: 'Node', propertyNames: ['name'], isUnique: false },
  { name: 'idx_person_email', entityType: 'Node', propertyNames: ['email'], isUnique: true },
]

export function SchemaPage() {
  return (
    <div className="flex h-full flex-col gap-4">
      <h1 className="text-2xl font-bold">Schema Browser</h1>
      <div className="grid gap-6 lg:grid-cols-2">
        <div className="rounded-lg border border-border bg-card p-4">
          <LabelList
            nodeLabels={DEMO_LABELS.nodeLabels}
            edgeLabels={DEMO_LABELS.edgeLabels}
          />
        </div>
        <div className="rounded-lg border border-border bg-card p-4">
          <IndexManager indexes={DEMO_INDEXES} />
        </div>
      </div>
    </div>
  )
}
