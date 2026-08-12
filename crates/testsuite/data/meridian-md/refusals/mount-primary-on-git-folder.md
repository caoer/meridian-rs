---
type: meridian-config
version: 1
---

# My system (refused: field-not-permitted-for-kind)

The primary root is where the fleet daemon writes — journal, locks, the
change feed's anchor. A git-folder root binds a source repo, so designating
one states an intent the daemon must never honour, and ignoring the line
would let the config assert a role nothing checks.

```meridian-mount
name: archive
path: /Users/Shared/repos/archive
kind: git-folder
primary: true
```
