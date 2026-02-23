import { useState } from 'react'

interface IndexInfo {
  name: string
  entityType: string
  propertyNames: string[]
  isUnique: boolean
}

interface IndexManagerProps {
  indexes: IndexInfo[]
  onCreateIndex?: (index: IndexInfo) => void
  onDropIndex?: (name: string) => void
}

function CreateIndexForm({
  onSubmit,
  onCancel,
}: {
  onSubmit: (index: IndexInfo) => void
  onCancel: () => void
}) {
  const [name, setName] = useState('')
  const [entityType, setEntityType] = useState('Node')
  const [propertyNames, setPropertyNames] = useState('')
  const [isUnique, setIsUnique] = useState(false)

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    if (!name.trim() || !propertyNames.trim()) return
    onSubmit({
      name: name.trim(),
      entityType,
      propertyNames: propertyNames.split(',').map((s) => s.trim()).filter(Boolean),
      isUnique,
    })
  }

  return (
    <form
      onSubmit={handleSubmit}
      className="rounded-lg border border-border bg-secondary/30 p-4"
    >
      <h3 className="mb-3 text-sm font-semibold text-foreground">Create Index</h3>
      <div className="flex flex-col gap-3">
        <div className="flex flex-col gap-1">
          <label className="text-xs text-muted-foreground">Name</label>
          <input
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="idx_label_property"
            className="rounded border border-border bg-background px-2.5 py-1.5 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring"
          />
        </div>
        <div className="flex flex-col gap-1">
          <label className="text-xs text-muted-foreground">Entity Type</label>
          <select
            value={entityType}
            onChange={(e) => setEntityType(e.target.value)}
            className="rounded border border-border bg-background px-2.5 py-1.5 text-sm text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
          >
            <option value="Node">Node</option>
            <option value="Edge">Edge</option>
          </select>
        </div>
        <div className="flex flex-col gap-1">
          <label className="text-xs text-muted-foreground">Property Names (comma-separated)</label>
          <input
            type="text"
            value={propertyNames}
            onChange={(e) => setPropertyNames(e.target.value)}
            placeholder="name, email"
            className="rounded border border-border bg-background px-2.5 py-1.5 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring"
          />
        </div>
        <label className="flex items-center gap-2 text-sm text-foreground">
          <input
            type="checkbox"
            checked={isUnique}
            onChange={(e) => setIsUnique(e.target.checked)}
            className="rounded border-border"
          />
          Unique
        </label>
        <div className="flex gap-2">
          <button
            type="submit"
            className="rounded bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90"
          >
            Create
          </button>
          <button
            type="button"
            onClick={onCancel}
            className="rounded border border-border px-3 py-1.5 text-sm text-foreground hover:bg-secondary"
          >
            Cancel
          </button>
        </div>
      </div>
    </form>
  )
}

export function IndexManager({ indexes, onCreateIndex, onDropIndex }: IndexManagerProps) {
  const [showCreate, setShowCreate] = useState(false)
  const [confirmDrop, setConfirmDrop] = useState<string | null>(null)
  const [items, setItems] = useState(indexes)

  const handleCreate = (index: IndexInfo) => {
    setItems((prev) => [...prev, index])
    onCreateIndex?.(index)
    setShowCreate(false)
  }

  const handleDrop = (name: string) => {
    setItems((prev) => prev.filter((idx) => idx.name !== name))
    onDropIndex?.(name)
    setConfirmDrop(null)
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold uppercase tracking-wider text-muted-foreground">
          Indexes
        </h2>
        {!showCreate && (
          <button
            onClick={() => setShowCreate(true)}
            className="rounded bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground hover:bg-primary/90"
          >
            Create Index
          </button>
        )}
      </div>

      {showCreate && (
        <CreateIndexForm
          onSubmit={handleCreate}
          onCancel={() => setShowCreate(false)}
        />
      )}

      <div className="overflow-auto rounded-lg border border-border">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-border bg-secondary/50">
              <th className="px-3 py-2 text-left font-medium text-foreground">Name</th>
              <th className="px-3 py-2 text-left font-medium text-foreground">Type</th>
              <th className="px-3 py-2 text-left font-medium text-foreground">Properties</th>
              <th className="px-3 py-2 text-left font-medium text-foreground">Unique</th>
              <th className="px-3 py-2 text-right font-medium text-foreground">Actions</th>
            </tr>
          </thead>
          <tbody>
            {items.length === 0 && (
              <tr>
                <td colSpan={5} className="px-3 py-4 text-center text-muted-foreground">
                  No indexes defined
                </td>
              </tr>
            )}
            {items.map((idx) => (
              <tr key={idx.name} className="border-b border-border last:border-0 hover:bg-secondary/30">
                <td className="px-3 py-2 font-mono text-xs">{idx.name}</td>
                <td className="px-3 py-2">{idx.entityType}</td>
                <td className="px-3 py-2">
                  <div className="flex flex-wrap gap-1">
                    {idx.propertyNames.map((p) => (
                      <span key={p} className="rounded bg-secondary px-1.5 py-0.5 text-xs">
                        {p}
                      </span>
                    ))}
                  </div>
                </td>
                <td className="px-3 py-2">
                  {idx.isUnique && (
                    <span className="rounded bg-primary/20 px-1.5 py-0.5 text-xs font-medium text-primary">
                      unique
                    </span>
                  )}
                </td>
                <td className="px-3 py-2 text-right">
                  {confirmDrop === idx.name ? (
                    <span className="flex items-center justify-end gap-2">
                      <span className="text-xs text-muted-foreground">Confirm?</span>
                      <button
                        onClick={() => handleDrop(idx.name)}
                        className="rounded bg-destructive px-2 py-1 text-xs font-medium text-foreground hover:bg-destructive/90"
                      >
                        Drop
                      </button>
                      <button
                        onClick={() => setConfirmDrop(null)}
                        className="rounded border border-border px-2 py-1 text-xs text-foreground hover:bg-secondary"
                      >
                        Cancel
                      </button>
                    </span>
                  ) : (
                    <button
                      onClick={() => setConfirmDrop(idx.name)}
                      className="rounded border border-border px-2 py-1 text-xs text-destructive hover:bg-destructive/10"
                    >
                      Drop
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  )
}
