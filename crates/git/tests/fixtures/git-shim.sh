#!/bin/sh
# Argv-logging git shim for the plumbing gates: log the argv, then exec the
# real git. Checked in and never written by the test process — a test that
# writes an executable and immediately execs it races every sibling libtest
# thread that forks, and Linux answers ETXTBSY (measured 291/2000 rounds).
#
# The log lands in the repository the handle points at: every `Repo` call is
# `git -C <root> …`, so "$2" is that root.
printf '%s\n' "$*" >> "$2/git-argv.log"
exec git "$@"
