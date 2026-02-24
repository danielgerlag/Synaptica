import { useEffect, useRef } from 'react'
import { Moon, Sun, Wifi, WifiOff } from 'lucide-react'
import { useAppStore } from '@/lib/store'
import { client } from '@/lib/grpc-client'

export function Header() {
  const { theme, toggleTheme, isConnected, setConnected } = useAppStore()
  const intervalRef = useRef<ReturnType<typeof setInterval>>(undefined)

  useEffect(() => {
    const check = () => {
      client.health()
        .then(() => setConnected(true))
        .catch(() => setConnected(false))
    }
    check()
    intervalRef.current = setInterval(check, 10_000)
    return () => clearInterval(intervalRef.current)
  }, [setConnected])

  return (
    <header className="flex h-14 items-center justify-between border-b border-border bg-card px-6">
      <div className="flex items-center gap-2">
        {isConnected ? (
          <Wifi className="h-4 w-4 text-green-500" />
        ) : (
          <WifiOff className="h-4 w-4 text-red-500" />
        )}
        <span className="text-sm text-muted-foreground">
          {isConnected ? 'Connected' : 'Disconnected'}
        </span>
      </div>
      <button
        onClick={toggleTheme}
        className="rounded-md p-2 hover:bg-accent"
        aria-label="Toggle theme"
      >
        {theme === 'dark' ? (
          <Sun className="h-4 w-4" />
        ) : (
          <Moon className="h-4 w-4" />
        )}
      </button>
    </header>
  )
}
