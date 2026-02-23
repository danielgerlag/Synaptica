import { NavLink } from 'react-router-dom'
import {
  Terminal,
  Share2,
  Database,
  Compass,
  Server,
  BarChart3,
} from 'lucide-react'
import { cn } from '@/lib/utils'

const navItems = [
  { to: '/query', icon: Terminal, label: 'Query' },
  { to: '/graph', icon: Share2, label: 'Graph' },
  { to: '/schema', icon: Database, label: 'Schema' },
  { to: '/explorer', icon: Compass, label: 'Explorer' },
  { to: '/cluster', icon: Server, label: 'Cluster' },
  { to: '/metrics', icon: BarChart3, label: 'Metrics' },
]

export function Sidebar() {
  return (
    <aside className="flex h-full w-16 flex-col items-center border-r border-border bg-card py-4 lg:w-56">
      <div className="mb-8 flex items-center gap-2 px-4">
        <Share2 className="h-6 w-6 text-primary" />
        <span className="hidden text-lg font-bold lg:block">Synaptica</span>
      </div>
      <nav className="flex flex-1 flex-col gap-1 px-2">
        {navItems.map(({ to, icon: Icon, label }) => (
          <NavLink
            key={to}
            to={to}
            className={({ isActive }) =>
              cn(
                'flex items-center gap-3 rounded-md px-3 py-2 text-sm font-medium transition-colors',
                'hover:bg-accent hover:text-accent-foreground',
                isActive
                  ? 'bg-accent text-accent-foreground'
                  : 'text-muted-foreground'
              )
            }
          >
            <Icon className="h-4 w-4 shrink-0" />
            <span className="hidden lg:block">{label}</span>
          </NavLink>
        ))}
      </nav>
    </aside>
  )
}
