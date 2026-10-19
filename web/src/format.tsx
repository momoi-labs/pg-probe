const NBSP = ' '

const IEC = ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB']

export function bytes(value: number): { number: string; unit: string } {
  let scaled = value
  let index = 0
  while (Math.abs(scaled) >= 1024 && index < IEC.length - 1) {
    scaled /= 1024
    index += 1
  }
  return { number: index === 0 ? String(scaled) : scaled.toFixed(1), unit: IEC[index] }
}

export function Bytes({ value }: { value: number }) {
  const { number, unit } = bytes(value)
  return (
    <>
      {number}
      {NBSP}
      <span className="muted">{unit}</span>
    </>
  )
}

export function Percent({ value }: { value: number }) {
  return (
    <>
      {value.toFixed(1)}
      <span className="muted">%</span>
    </>
  )
}

// Unknown: no value is known. The reason is also shown as adjacent text by the
// caller, so it does not live only in the accessible label.
export function Unknown({ reason }: { reason: string }) {
  return (
    <span className="muted" aria-label={`Unknown, ${reason}`}>
      —
    </span>
  )
}

export function utcMinute(iso: string) {
  return `${iso.slice(0, 10)} ${iso.slice(11, 16)} UTC`
}

export function duration(fromIso: string, toIso: string) {
  const ms = Date.parse(toIso) - Date.parse(fromIso)
  if (Number.isNaN(ms)) return null
  if (ms < 1000) return `${ms}${NBSP}ms`
  const seconds = ms / 1000
  if (seconds < 60) return `${seconds.toFixed(1)}${NBSP}s`
  return `${Math.round(seconds / 60)}${NBSP}min`
}
