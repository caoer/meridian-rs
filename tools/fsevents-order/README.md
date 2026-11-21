# fsevents-order

The instruments behind `docs/node-rev-merkle-spec.md` §6.4, the stream that
can vouch: on macOS, does an FSEvents stream deliver a sentinel's entry only
after the entries of every write that completed before the sentinel existed?
It does not. macOS only; CI does not build these.

```sh
clang -O2 -o fsev-probe fsev-probe.c -framework CoreServices
clang -O2 -o kfse-tap kfse-tap.c

# three streams at once, each in its own directory
for n in 1 2 3; do mkdir -p /abs/scratch/p$n; ./fsev-probe /abs/scratch/p$n 100000 trio > p$n.out & done
grep -c ANOMALY p?.out

# the kernel's own order beside it (root; one clone watcher on /dev/fsevents)
sudo ./kfse-tap /scratch/p > kernel.log
```

`fsev-probe` opens the stream as notify does (`SinceNow`, latency 0,
`FileEvents|NoDefer`, a run loop on its own thread) and runs one cookie
barrier per iteration: complete a member write, create a sentinel, wait for
the sentinel's entry, then ask whether the member's entry has arrived. An
`ANOMALY` block is one barrier where it had not, with the entries around it:
batch, index in the batch, event id, flags, path.

`kfse-tap` prints the kernel queue's events for paths containing its argument,
in the order the kernel hands them to `fseventsd`. For the same barrier it
shows the member events ahead of the sentinel's create, which is the order the
stream then breaks.

Each probe paces itself (`gap_us`). Six at once put `fseventsd` near one full
core for the length of the run; it returns to idle when they stop.
