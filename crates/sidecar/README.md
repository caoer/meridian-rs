# sidecar

**Charter:** thin NDJSON stdin/stdout bin over `meridian` + `wire` — arg parsing and
transport wiring only; anything a lib could own, a lib owns (cargo doctrine: each op
is a thin wrapper around a lib call). Budget: <300 lines including main.
Respawn-friendly by law 2: no state exists to lose.

**Three laws:** (1) Rust is the world model, Go is the actor — this bin answers ops,
never initiates. (2) Disk = markdown, the only durable truth; sidecar memory is
disposable, restart is full recovery. (3) One-way arrows: files → Rust → Go.
