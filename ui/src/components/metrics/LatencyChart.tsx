import { LineChart, Line, XAxis, YAxis, CartesianGrid, Tooltip, Legend, ResponsiveContainer } from 'recharts'

interface LatencyChartProps {
  data: { time: string; p50: number; p95: number; p99: number }[]
}

export function LatencyChart({ data }: LatencyChartProps) {
  return (
    <div className="rounded-lg border border-border bg-card p-4 flex flex-col gap-3">
      <h2 className="text-lg font-semibold text-foreground">Latency (ms)</h2>
      <ResponsiveContainer width="100%" height={250}>
        <LineChart data={data}>
          <CartesianGrid strokeDasharray="3 3" stroke="#27272a" />
          <XAxis dataKey="time" tick={{ fill: '#a1a1aa', fontSize: 12 }} />
          <YAxis tick={{ fill: '#a1a1aa', fontSize: 12 }} />
          <Tooltip
            contentStyle={{ backgroundColor: '#09090b', border: '1px solid #27272a', borderRadius: 8 }}
            labelStyle={{ color: '#a1a1aa' }}
          />
          <Legend wrapperStyle={{ color: '#a1a1aa' }} />
          <Line type="monotone" dataKey="p50" name="p50" stroke="#3b82f6" strokeWidth={2} dot={false} />
          <Line type="monotone" dataKey="p95" name="p95" stroke="#eab308" strokeWidth={2} dot={false} />
          <Line type="monotone" dataKey="p99" name="p99" stroke="#ef4444" strokeWidth={2} dot={false} />
        </LineChart>
      </ResponsiveContainer>
    </div>
  )
}
