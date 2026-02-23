import { useState } from 'react'

type PropertyType = 'string' | 'integer' | 'float' | 'boolean'

export interface PropertyRow {
  key: string
  value: string
  type: PropertyType
}

interface PropertyEditorProps {
  properties: PropertyRow[]
  onChange: (properties: PropertyRow[]) => void
}

export function PropertyEditor({ properties, onChange }: PropertyEditorProps) {
  const update = (index: number, field: keyof PropertyRow, value: string) => {
    const next = [...properties]
    next[index] = { ...next[index], [field]: value }
    onChange(next)
  }

  const remove = (index: number) => {
    onChange(properties.filter((_, i) => i !== index))
  }

  const add = () => {
    onChange([...properties, { key: '', value: '', type: 'string' }])
  }

  return (
    <div className="flex flex-col gap-2">
      {properties.map((prop, i) => (
        <div key={i} className="flex items-center gap-2">
          <input
            type="text"
            value={prop.key}
            onChange={(e) => update(i, 'key', e.target.value)}
            placeholder="key"
            className="w-32 rounded border border-border bg-background px-2 py-1.5 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring"
          />
          <input
            type="text"
            value={prop.value}
            onChange={(e) => update(i, 'value', e.target.value)}
            placeholder="value"
            className="flex-1 rounded border border-border bg-background px-2 py-1.5 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring"
          />
          <select
            value={prop.type}
            onChange={(e) => update(i, 'type', e.target.value)}
            className="rounded border border-border bg-background px-2 py-1.5 text-sm text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
          >
            <option value="string">string</option>
            <option value="integer">integer</option>
            <option value="float">float</option>
            <option value="boolean">boolean</option>
          </select>
          <button
            type="button"
            onClick={() => remove(i)}
            className="rounded border border-border px-2 py-1.5 text-sm text-destructive hover:bg-destructive/10"
          >
            ✕
          </button>
        </div>
      ))}
      <button
        type="button"
        onClick={add}
        className="self-start rounded border border-border px-3 py-1.5 text-xs text-foreground hover:bg-secondary"
      >
        + Add Property
      </button>
    </div>
  )
}
