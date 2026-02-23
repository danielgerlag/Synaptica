import { useRef, useEffect } from 'react'
import { EditorView, keymap, lineNumbers } from '@codemirror/view'
import { EditorState } from '@codemirror/state'
import { bracketMatching } from '@codemirror/language'
import { defaultKeymap } from '@codemirror/commands'
import { oneDark } from '@codemirror/theme-one-dark'
import { gqlLanguage } from './gql-language'

interface QueryEditorProps {
  value: string
  onChange: (v: string) => void
  onExecute: (query: string) => void
}

export function QueryEditor({ value, onChange, onExecute }: QueryEditorProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const viewRef = useRef<EditorView | null>(null)

  useEffect(() => {
    if (!containerRef.current) return

    const executeKeymap = keymap.of([
      {
        key: 'Ctrl-Enter',
        run: (view) => {
          onExecute(view.state.doc.toString())
          return true
        },
      },
    ])

    const updateListener = EditorView.updateListener.of((update) => {
      if (update.docChanged) {
        onChange(update.state.doc.toString())
      }
    })

    const theme = EditorView.theme({
      '&': {
        minHeight: '150px',
        height: '100%',
        backgroundColor: '#09090b',
      },
      '.cm-scroller': {
        overflow: 'auto',
      },
      '.cm-content': {
        fontFamily: 'ui-monospace, SFMono-Regular, "SF Mono", Menlo, monospace',
        fontSize: '14px',
      },
    })

    const state = EditorState.create({
      doc: value,
      extensions: [
        executeKeymap,
        keymap.of(defaultKeymap),
        lineNumbers(),
        bracketMatching(),
        gqlLanguage,
        oneDark,
        theme,
        updateListener,
      ],
    })

    const view = new EditorView({ state, parent: containerRef.current })
    viewRef.current = view

    return () => {
      view.destroy()
      viewRef.current = null
    }
    // Only create editor once
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // Sync external value changes into the editor
  useEffect(() => {
    const view = viewRef.current
    if (!view) return
    const current = view.state.doc.toString()
    if (current !== value) {
      view.dispatch({
        changes: { from: 0, to: current.length, insert: value },
      })
    }
  }, [value])

  return (
    <div
      ref={containerRef}
      className="h-full min-h-[150px] overflow-hidden rounded-lg border border-border"
    />
  )
}
