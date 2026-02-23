import { useState } from 'react'
import { PropertyEditor, type PropertyRow } from './PropertyEditor'

interface NodeFormProps {
  initialLabels?: string[]
  initialProperties?: PropertyRow[]
  onSave: (data: { labels: string[]; properties: PropertyRow[] }) => void
  onCancel: () => void
}

export function NodeForm({ initialLabels = [], initialProperties = [], onSave, onCancel }: NodeFormProps) {
  const [labels, setLabels] = useState<string[]>(initialLabels)
  const [labelInput, setLabelInput] = useState('')
  const [properties, setProperties] = useState<PropertyRow[]>(
    initialProperties.length > 0 ? initialProperties : [{ key: '', value: '', type: 'string' }]
  )

  const addLabel = () => {
    const trimmed = labelInput.trim()
    if (trimmed && !labels.includes(trimmed)) {
      setLabels([...labels, trimmed])
    }
    setLabelInput('')
  }

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') {
      e.preventDefault()
      addLabel()
    }
  }

  const removeLabel = (label: string) => {
    setLabels(labels.filter((l) => l !== label))
  }

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    onSave({ labels, properties })
  }

  return (
    <form onSubmit={handleSubmit} className="rounded-lg border border-border bg-card p-4">
      <h3 className="mb-4 text-sm font-semibold text-foreground">Create Node</h3>

      <div className="mb-4 flex flex-col gap-1">
        <label className="text-xs text-muted-foreground">Labels</label>
        <div className="flex flex-wrap items-center gap-1.5">
          {labels.map((label) => (
            <span
              key={label}
              className="flex items-center gap-1 rounded bg-primary/20 px-2 py-0.5 text-xs font-medium text-primary"
            >
              {label}
              <button
                type="button"
                onClick={() => removeLabel(label)}
                className="text-primary/60 hover:text-primary"
              >
                ✕
              </button>
            </span>
          ))}
          <input
            type="text"
            value={labelInput}
            onChange={(e) => setLabelInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="Type label, press Enter"
            className="min-w-[160px] flex-1 rounded border border-border bg-background px-2 py-1.5 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring"
          />
        </div>
      </div>

      <div className="mb-4 flex flex-col gap-1">
        <label className="text-xs text-muted-foreground">Properties</label>
        <PropertyEditor properties={properties} onChange={setProperties} />
      </div>

      <div className="flex gap-2">
        <button
          type="submit"
          className="rounded bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90"
        >
          Save
        </button>
        <button
          type="button"
          onClick={onCancel}
          className="rounded border border-border px-3 py-1.5 text-sm text-foreground hover:bg-secondary"
        >
          Cancel
        </button>
      </div>
    </form>
  )
}
