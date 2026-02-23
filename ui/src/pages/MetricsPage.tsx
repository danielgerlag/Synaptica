import { useState, useEffect, useCallback } from 'react'
import { MetricsDashboard, type MetricCard } from '@/components/metrics/MetricsDashboard'
import { ThroughputChart } from '@/components/metrics/ThroughputChart'
import { LatencyChart } from '@/components/metrics/LatencyChart'

function generateDemoData(points: number) {
  const now = Date.now()
  return Array.from({ length: points }, (_, i) => ({
    time: new Date(now - (points - i) * 5000).toLocaleTimeString(),
    throughput: 100 + Math.random() * 50,
    p50: 5 + Math.random() * 3,
    p95: 15 + Math.random() * 10,
    p99: 50 + Math.random() * 30,
  }))
}

const SUMMARY_CARDS: MetricCard[] = [
  { label: 'Queries/sec', value: '127', trend: 'up' },
  { label: 'Avg Latency', value: '12ms', trend: 'down' },
  { label: 'Active Connections', value: '34', trend: 'up' },
  { label: 'Storage Size', value: '2.4 GB', trend: 'up' },
]

export function MetricsPage() {
  const [autoRefresh, setAutoRefresh] = useState(false)
  const [data, setData] = useState(() => generateDemoData(60))

  const refresh = useCallback(() => {
    setData(generateDemoData(60))
  }, [])

  useEffect(() => {
    if (!autoRefresh) return
    const id = globalThis.setInterval(refresh, 5000)
    return () => globalThis.clearInterval(id)
  }, [autoRefresh, refresh])

  const throughputData = data.map((d) => ({ time: d.time, value: d.throughput }))
  const latencyData = data.map((d) => ({ time: d.time, p50: d.p50, p95: d.p95, p99: d.p99 }))

  return (
    <div className="flex h-full flex-col gap-4">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">Metrics Dashboard</h1>
        <button
          onClick={() => setAutoRefresh((v) => !v)}
          className={`rounded px-3 py-1 text-sm font-medium ${
            autoRefresh
              ? 'bg-primary text-primary-foreground'
              : 'border border-border bg-card text-foreground'
          }`}
        >
          {autoRefresh ? 'Auto-Refresh ON' : 'Auto-Refresh OFF'}
        </button>
      </div>

      <MetricsDashboard cards={SUMMARY_CARDS} />
      <ThroughputChart data={throughputData} />
      <LatencyChart data={latencyData} />
    </div>
  )
}
