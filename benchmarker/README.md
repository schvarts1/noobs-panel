# benchmarker

A workload that pushes against the limits of an instance.

The panel gives every instance a memory, a processor time and a disk limit.
The benchmarker exists to show that those limits are real: it serves a page
with what the instance is using, and it can be asked to use as much of every
resource as it can.

## Running

```sh
cargo run --release -p benchmarker -- --port 8100
```

The port is the preferred one; the nearest free port is used when it is
busy.

## What it serves

| Address | What it does |
| ------- | ------------ |
| `/` | The usage of the instance and the limits the kernel gave the process |
| `/health` | A short answer, for a check |
| `/stress/memory` | Takes as much memory as it can, until the kernel refuses |
| `/stress/disk` | Writes a file until the kernel refuses |
| `/stress/cpu` | Uses the processor until the kernel stops the instance |

The memory and the disk test report what they managed to use and then carry
on, so the instance stays alive. The processor test ends the instance, because
that is the only way to see that the limit is real.
