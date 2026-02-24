import { useState, useEffect, useCallback, useRef } from 'react'
import { client, type MetricsData } from '@/lib/grpc-client'
import { MetricsDashboard, type MetricCard } from '@/components/metrics/MetricsDashboard'
import { ThroughputChart } from '@/components/metrics/ThroughputChart'
import { LatencyChart } from '@/components/metrics/LatencyChart'

interface TimePoint {
  time: string
  throughput: number
  p50: number
  p95: number
  p99: number
}

const MAX_POINTS = 60

export function MetricsPage() {
  const [autoRefresh, setAutoRefresh] = useState(false)
  const [timeSeries, setTimeSeries] = useState<TimePoint[]>([])
  const [cards, setCards] = useState<MetricCard[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string>()
  const prevMetrics = useRef<MetricsData | null>(null)

  const fetchMetrics = useCallback(async () => {
    try {
      const m = await client.getMetrics()
      const now = new Date().toLocaleTimeString()

      const newCards: MetricCard[] = []
      const qps = m.gauges['queries_per_second'] ?? m.counters['queries_total'] ?? 0
      const avgLat = m.gauges['avg_latency_ms'] ?? m.gauges['query_latency_p50'] ?? 0
      const connections = m.gauges['active_connections'] ?? m.counters['connections_total'] ?? 0
      const storage = m.gauges['storage_bytes'] ?? 0

      const prev = prevMetrics.current
      const trend = (key: string): 'up' | 'down' => {
        if (!prev) return 'up'
        const pv = prev.gauges[key] ?? prev.counters[key] ?? 0
        const cv = m.gauges[key] ?? m.counters[key] ?? 0
        return cv >= pv ? 'up' : 'down'
      }

      newCards.push({ label: 'Queries/sec', value: qps.toFixed(0), trend: trend('queries_per_second') })
      newCards.push({ label: 'Avg Latency', value: `${avgLat.toFixed(1)}ms`, trend: trend('avg_latency_ms') })
      newCards.push({ label: 'Active Connections', value: connections.toFixed(0), trend: trend('active_connections') })
      if (storage > 0) {
        const mb = storage / (1024 * 1024)
        newCards.push({ label: 'Storage Size', value: mb > 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${mb.toFixed(1)} MB`, trend: trend('storage_bytes') })
      } else {
        newCards.push({ label: 'Storage Size', value: '—', trend: 'up' })
      }

      setCards(newCards)
      prevMetrics.current = m

      const point: TimePoint = {
        time: now,
        throughput: qps,
        p50: m.gauges['query_latency_p50'] ?? avgLat,
        p95: m.gauges['query_latency_p95'] ?? avgLat * 2,
        p99: m.gauges['query_latency_p99'] ?? avgLat * 4,
      }
      setTimeSeries((prev) => [...prev, point].slice(-MAX_POINTS))
      setError(undefined)
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to fetch metrics')
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => { fetchMetrics() }, [fetchMetrics])

  useEffect(() => {
    if (!autoRefresh) return
    const id = globalThis.setInterval(fetchMetrics, 5000)
    return () => globalThis.clearInterval(id)
  }, [autoRefresh, fetchMetrics])

  const throughputData = timeSeries.map((d) => ({ time: d.time, value: d.throughput }))
  const latencyData = timeSeries.map((d) => ({ time: d.time, p50: d.p50, p95: d.p95, p99: d.p99 }))

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

      {error && (
        <div className="rounded-lg border border-red-500/30 bg-red-500/10 p-3 text-sm text-red-400">
          {error}
        </div>
      )}

      {loading ? (
        <div className="flex items-center gap-2 py-12 justify-center text-muted-foreground">
          <span className="inline-block h-4 w-4 animate-spin rounded-full border-2 border-current border-t-transparent" />
          Loading metrics…
        </div>
      ) : (
        <>
          <MetricsDashboard cards={cards.length > 0 ? cards : [
            { label: 'Queries/sec', value: '—', trend: 'up' },
            { label: 'Avg Latency', value: '—', trend: 'up' },
            { label: 'Active Connections', value: '—', trend: 'up' },
            { label: 'Storage Size', value: '—', trend: 'up' },
          ]} />
          {timeSeries.length > 0 ? (
            <>
              <ThroughputChart data={throughputData} />
              <LatencyChart data={latencyData} />
            </>
          ) : (
            <p className="text-muted-foreground text-center py-8">No metrics data yet. Enable auto-refresh to collect time series.</p>
          )}
        </>
      )}
    </div>
  )
}
