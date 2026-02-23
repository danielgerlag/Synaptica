import { cn } from '@/lib/utils'

export interface MetricCard {
  label: string
  value: string
  trend: 'up' | 'down'
}

interface MetricsDashboardProps {
  cards: MetricCard[]
}

export function MetricsDashboard({ cards }: MetricsDashboardProps) {
  return (
    <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">
      {cards.map((card) => (
        <div
          key={card.label}
          className="rounded-lg border border-border bg-card p-4 flex flex-col gap-1"
        >
          <span className="text-xs text-muted-foreground">{card.label}</span>
          <div className="flex items-center gap-2">
            <span className="text-2xl font-bold text-foreground">{card.value}</span>
            <span
              className={cn(
                'text-sm',
                card.trend === 'up' ? 'text-green-500' : 'text-red-500',
              )}
            >
              {card.trend === 'up' ? '↑' : '↓'}
            </span>
          </div>
        </div>
      ))}
    </div>
  )
}
