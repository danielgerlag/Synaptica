import { create } from 'zustand'

export interface QueryHistoryEntry {
  id: string
  query: string
  timestamp: number
  executionTimeMs?: number
  rowCount?: number
  error?: string
}

interface AppState {
  // Connection
  serverUrl: string
  isConnected: boolean
  setServerUrl: (url: string) => void
  setConnected: (connected: boolean) => void

  // Query history
  queryHistory: QueryHistoryEntry[]
  addQueryHistory: (entry: QueryHistoryEntry) => void
  clearQueryHistory: () => void

  // Current graph
  currentGraph: string
  setCurrentGraph: (graph: string) => void

  // Theme
  theme: 'dark' | 'light'
  toggleTheme: () => void
}

export const useAppStore = create<AppState>((set) => ({
  serverUrl: 'http://localhost:9090',
  isConnected: false,
  setServerUrl: (url) => set({ serverUrl: url }),
  setConnected: (connected) => set({ isConnected: connected }),

  queryHistory: JSON.parse(localStorage.getItem('synaptica-query-history') || '[]'),
  addQueryHistory: (entry) =>
    set((state) => {
      const history = [entry, ...state.queryHistory].slice(0, 100)
      localStorage.setItem('synaptica-query-history', JSON.stringify(history))
      return { queryHistory: history }
    }),
  clearQueryHistory: () => {
    localStorage.removeItem('synaptica-query-history')
    set({ queryHistory: [] })
  },

  currentGraph: 'default',
  setCurrentGraph: (graph) => set({ currentGraph: graph }),

  theme: (localStorage.getItem('synaptica-theme') as 'dark' | 'light') || 'dark',
  toggleTheme: () =>
    set((state) => {
      const next = state.theme === 'dark' ? 'light' : 'dark'
      localStorage.setItem('synaptica-theme', next)
      document.documentElement.classList.toggle('dark', next === 'dark')
      return { theme: next }
    }),
}))
