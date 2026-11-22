---
type: spec
id: fsevents-helper
status: standing
description: The macOS ordered event source — a root helper that reads `/dev/fsevents` in kernel order and serves each engine the events under its workspace root, so the currency barrier is a proof on macOS too.
owns: [the helper's privilege and socket, the helper handshake, the helper record stream, the helper source's qualification and loss rules]
---

# The macOS ordered event source

> Standing law: `node-rev-merkle-spec.md` §6.4 (the event feed, the currency
> barrier, the stream that can vouch) and §6.7 (which consumer asks at which
> grade). This file rules the one piece those sections name and do not
> describe: the helper process, its socket, and when the engine trusts it.

On macOS the FSEvents stream does not prove order (`merkle §6.4`, the stream
that can vouch), so a guard-grade observation there takes the extent-refresh
floor: one `lstat` per member, about 0.2 s on a 222k-member root, on every
write. The kernel's own queue is ordered. `/dev/fsevents` hands a watcher its
events in the order the kernel raised them, and opening it needs root and
nothing else. `mrd-fsevents-helper` is that watcher. With it running, a
barrier on macOS is the proof it is on inotify; without it, nothing changes.

## 1. The helper

One process, started by launchd as root (`app.0xdao.mrd-fsevents-helper`, a
LaunchDaemon; binary `mrd-fsevents-helper`, crate `fsevents-helper`). It does
one thing: it reads one clone watcher on `/dev/fsevents` and writes, to each
subscriber, the events whose path lies under that subscriber's root. It opens
no network socket, reads no configuration, writes no file but its socket and
the empty files of §3, and depends on `libc` alone. It is one thread around one `poll`.

- **Root to set up.** Root opens the device, binds the socket under
  `/var/run` and makes the helper's own directory there. The helper then
  becomes `nobody`, user and group, and cannot return: reading the watcher,
  accepting, and inspecting a passed descriptor need no privilege. `nobody`
  is not the helper's alone: another process running as it could signal the
  helper, which costs engines the helper. It cannot forge a fence: the helper
  counts a creation in its directory only when the kernel names the helper as
  the process that raised it.

- **The watcher.** One `FSEVENTS_CLONE` for the event types that can move a
  member or hide one: create, delete, rename, content modified, exchange,
  stat, owner and extended-attribute changes, directory create, clone. The
  kernel has eight watcher slots for the whole machine; the helper takes one,
  for every engine and every root. A full table fails the clone and the
  helper exits.
- **The queue.** Depth 1024. The kernel's event pool is shared by every
  watcher on the machine. A watcher that is not an Apple system service is
  flushed when its queue passes three quarters, and its next read begins with
  a dropped report, so a stalled helper holds at most 768 pooled events and
  costs the machine nothing else.
- **A read that returns.** The kernel's read skips the events it withholds
  (§4) and then sleeps for the next one, even after it has copied others,
  and that sleep can miss its wakeup: an event already copied then waits for
  the next event on the machine. The helper bounds each read with a 200 µs
  alarm, which returns what was copied.
- **Unsupported by Apple.** The kernel logs `Using /dev/fsevents directly is
  unsupported` when the watcher is added. A release that closes the device to
  root leaves the helper unable to start, which is the absent-helper case of
  §5.

## 2. The socket and who may subscribe

`/var/run/mrd-fsevents.sock`, a stream socket, mode `0666`. `/var/run` is
root's, so only root can bind that name; the engine connects only when the
socket file's owner is root or the engine's own user.

Any local user may connect. A subscription is granted for one directory, and
the client proves its claim by passing an open descriptor of that directory
with its hello. The helper requires that the descriptor is a directory and
that the connecting user owns it (root may subscribe to any), and takes the
directory's kernel path from the descriptor, never from a string the client
sent. A subscriber receives the type, the node kind and the root-relative path
of each event under its directory: no content, no process id, no path outside
it. Names inside a subdirectory the owner cannot list do reach the owner of
the root above it.

At most 64 connections; one that has not completed its hello within 5 s is
closed. A user who fills the table denies the helper to the others, and they
take the floor.

## 3. The handshake and the stream

The client sends the 8 bytes `MRDFSE1\n` in one message that carries the
root's descriptor (`SCM_RIGHTS`). The helper answers `ok\n`, or `no <reason>\n`
and closes: `bad-hello`, `not-a-directory`, `not-owner`, `no-path`,
`ignored-directory` (§4).

After `ok` the helper sends records, in the order it read the events from the
device, and fences (below):

| Field | Size | Meaning |
|---|---|---|
| `kind` | 1 | the kernel event type (`FSE_CREATE_FILE` 0, `FSE_DELETE` 1, `FSE_STAT_CHANGED` 2, `FSE_RENAME` 3, `FSE_CONTENT_MODIFIED` 4, `FSE_EXCHANGE` 5, `FSE_FINDER_INFO_CHANGED` 6, `FSE_CREATE_DIR` 7, `FSE_CHOWN` 8, `FSE_XATTR_MODIFIED` 9, `FSE_XATTR_REMOVED` 10, `FSE_CLONE` 14), or `0xFF`: events were lost |
| `node` | 1 | 0 unknown, 1 regular file, 2 directory, 3 other |
| `len` | 2, little-endian | length of `path` |
| `path` | `len` | the path relative to the subscribed root; empty for the root itself and for `0xFF` |

A rename and an exchange send one record for each of their two paths that
lies under the root; a clone sends its destination. `0xFE` is a fence. A loss record is sent
when the kernel reports dropped events, and to a subscriber whose own output
passed 1 MiB unread: the helper never waits on a subscriber, it discards that
subscriber's backlog and says so.

The helper ends a subscription when the root directory is deleted, or when it
or a directory above it is renamed: the path the subscription matches no
longer names the tree. A subscriber also receives the events the helper had
not yet read when it subscribed, so a root deleted and created again just
before its subscription can lose that subscription to its own earlier
deletion; the feed is then on the FSEvents stream until its next attempt.

**The barrier.** The client writes a serial, 8 bytes little-endian. The
helper then creates a file in a directory of its own
(`/var/run/mrd-fsevents/`, `nobody`'s, mode `0700`), reads the device until
that creation comes back, and sends the client a fence: `kind` `0xFE`, the
serial in `path`. A fence may answer several serials at once and carries the
highest.

What the fence proves rests on two orders. The creation is raised after the
request arrived, and the kernel queues events as raised, so every event
raised before the request is ahead of the creation in the helper's queue.
And the helper reads on one thread, so the read that returns the creation
began after it was raised, and a read reports first any loss the kernel
flagged before it began. Every event raised before the request is therefore
ahead of the fence in the client's stream, or a loss record is.

A sentinel file cannot give the second half. The kernel flags a loss when
its event pool is exhausted, without touching the watcher's queue, and a
read already copying events goes on to copy the ones raised after the loss:
those reach the client first and the loss record follows in the next read. A
sentinel among them would be sighted with an earlier write missing and no
loss yet told.

The fence also spares the kernel's wake delay, 10 ms after an event or at
once past 16 pending: the helper reads as soon as it has raised its own
event. A barrier takes about 0.2 ms against about 12 ms for an event the
helper is left to be woken for, in a macOS 26.6 guest. A creation the kernel
dropped never comes back, so the helper raises another every 2 ms while a
fence waits.

## 4. What the engine does with it

The engine's feed (`merkle §6.4`) has one source at a time: the FSEvents
stream, or the helper. Events from the other are discarded.

**Qualifying.** A feed arms its FSEvents stream, as on a Mac with no helper,
and then tries the helper once before it is published, so a feed that can be
on the helper is on it at its first observation. It subscribes, and before it
trusts the helper it walks the root for a directory named `Backups.backupdb`
on a path with no dot-prefixed segment: the kernel withholds, from a watcher
that is not an Apple system service, every event under `/.Spotlight-V100/`,
`/.MobileBackups/` and `/Backups.backupdb/`. The first two are dot-prefixed
and so outside the hash domain; the third could hold members. The helper
refuses a root whose own path contains any of the three. A clean walk
switches the source to the helper under a missed-event collapse, so the next
observation is one floor pass and covers whatever the FSEvents stream had not
yet delivered. At the start that pass is the cold entry's own.

**Staying qualified.** The subscription began before the walk, so a
`Backups.backupdb` created after the walker passed is in the stream. A record
whose path has that segment, and a directory that arrives whole (a rename or
a clone) and holds one, or cannot be listed, or holds more than 1024 entries,
sends the feed back to the FSEvents stream.

**The barrier.** On the helper source a barrier writes no sentinel: the feed
sends its serial and waits for the fence that answers it (§3). `Seen` is
then a proof, and the write door, the priming cycle and the backstop cycle
ask the barrier as they do on inotify. The named-document check of
`merkle §6.7` does not run. A sentinel written before the source changed
vouches nothing on the helper source, and each change of source collapses
the barrier that was waiting across it.

**What a write is, here.** The kernel raises a content event when the writer
closes the file, and not before: a process that writes and holds the
descriptor raises nothing until it closes. On the helper's stream a completed
write is one whose descriptor is closed; bytes written through a descriptor
still open are announced at its close. The floor, which compares `lstat`
keys, sees them earlier.

**Consumed events and the kernel's coalescing.** The kernel drops an event
that repeats the previous coalescible one (same type, same process, same
file, within a second) while no watcher has read past it. A feed that has
already taken and applied the first would never hear of the second. So when
the helper is the source, every take that returns something first writes one
byte to `.meridian/cookies/take`: an event of the engine's own now
follows everything the take consumed, and a later write to any of those
members cannot repeat the previous event. The name carries no token and no
serial, so it is no sighting.

**Loss.** A loss record is the overflow cause of the rescan ladder: one floor
pass, and the helper remains the source.

## 5. Without the helper

The source is the FSEvents stream and every rule of `merkle §6.4` for a
stream that does not prove order holds unchanged: the door takes the floor,
a read checks the document it names. That is the state when the socket is
absent, refuses, belongs to another user, or answers `no`; when the walk
finds an ignored directory or cannot finish; and from the moment a
subscription ends, for any reason, including the helper's death. Leaving the
helper is a missed-event collapse. No caller sees an error: a barrier in
flight answers unproven and its observation takes the floor.

The feed tries again when a write door finds it on the FSEvents stream and
60 s have passed since the last attempt. There is no timer: a Mac with no
helper pays one failed `connect` per minute of write activity.

`MRD_FSEVENTS_SOCKET` names another socket path for the engine; set empty, the
engine tries no helper.

## 6. Installing

The LaunchDaemon's property list is `crates/fsevents-helper/app.0xdao.mrd-fsevents-helper.plist`;
it runs `/usr/local/libexec/mrd-fsevents-helper`. Installing it is an
operator's act on each Mac: copy the binary, copy the list to
`/Library/LaunchDaemons/`, `launchctl bootstrap system` it. The helper needs
no Full Disk Access, no entitlement and no signature beyond the ad-hoc one
the linker writes; a subscription to a directory under `~/Documents` works
without one. Engines find it at their next attempt.
