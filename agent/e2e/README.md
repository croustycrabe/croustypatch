# Real apt scenario (manual)

The apt backend runs real commands, so the parsers are unit-tested but the behaviour needs a Debian/Ubuntu system.
`apt-prep.sh` builds, inside an `ubuntu:24.04` container, three demo packages in the three situations a restore point
has to handle after an update is published:

| Package | Old version after the update | Restore source |
|---|---|---|
| `cdemo-a` | only in apt's cache | copy of the cached `.deb` |
| `cdemo-b` | still listed in the repository | `apt-get install name=version` |
| `cdemo-c` | nowhere (like `base-files` after a same-pocket update) | `dpkg-repack` |

How it was run: start the container (`--network host`), run `apt-prep.sh`, install the agent with the generated
installer, start the agent with `setpriv --bounding-set -setuid,-setgid` (no `CAP_SETUID`/`CAP_SETGID`, the cause of
`seteuid 42 failed`), then patch the three packages from the API: refused without `dpkg-repack`, succeeds with it, and
a rollback restores all three. Remember to remove Ubuntu's `/etc/apt/apt.conf.d/docker-clean`, which empties apt's
cache and would hide tier 1.
