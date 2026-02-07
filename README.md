# 🧱 slab-alloc

A from-scratch **slab allocator** written in Rust, implementing the `GlobalAlloc` trait. Designed for learning, experimentation, and use in systems programming projects.

Slab allocation is a memory management technique [originally developed by Jeff Bonwick](https://www.usenix.org/legacy/publications/library/proceedings/bos94/full_papers/bonwick.ps) for the SunOS 5.4 kernel. It eliminates fragmentation within size classes and provides **O(1) allocation and deallocation** by pre-partitioning memory into fixed-size slots.

## How It Works

```
                        SlabAllocator
                             │
        ┌────────────┬───────┴───────┬────────────┐
     8-byte       16-byte        64-byte       1024-byte
    SizeClass    SizeClass      SizeClass      SizeClass
        │            │              │              │
      Slab ──→    Slab ──→       Slab           Slab
      (4KB)      (4KB)          (4KB)           (4KB)
        │
   ┌────┴─────────────────────────────────┐
   │ [used] [free]→[free]→[free] [used]   │
   └──────────────────────────────────────┘
          intrusive free list
```

Each **size class** (8, 16, 32, 64, 128, 256, 512, 1024 bytes) maintains a linked list of **slabs** — 4 KB pages divided into fixed-size slots. Free slots form an **intrusive linked list**, meaning the next-pointer is stored inside the free slot itself, requiring zero external bookkeeping.

Allocations larger than 1024 bytes fall through to the system allocator.

## Features

- **O(1) alloc/dealloc** — just a linked list pop/push per operation
- **Zero metadata overhead** — free list pointers live inside unused slots
- **Thread-safe** — protected by an atomic spin lock (re-entrancy safe)
- **Configurable size classes** — easily adjust slot sizes and slab page size
- **System fallback** — large allocations transparently use the system allocator
- **Drop-in replacement** — implements `GlobalAlloc`, works with `#[global_allocator]`

## Quick Start

```rust
use slab_alloc::SlabAllocator;

#[global_allocator]
static ALLOCATOR: SlabAllocator = SlabAllocator::new();

fn main() {
    // Every heap allocation now goes through the slab allocator
    let mut data = Vec::new();
    for i in 0..1000 {
        data.push(i);
    }
    println!("Allocated {} items via slab allocator", data.len());

    let message = String::from("Hello from slab-alloc!");
    println!("{message}");
}
```

## Project Structure

```
slab-alloc/
├── src/
│   └── main.rs         # Full allocator implementation
├── Cargo.toml
└── README.md
```

## Architecture

### `SlabAllocator` (top level)

The global allocator entry point. Protects all internals with an atomic spin lock for thread safety and re-entrancy. Size classes are const-initialized at compile time. Routes each request to the appropriate size class based on the requested `Layout`. Falls back to the system allocator if the lock is already held (re-entrant call) or the request exceeds the largest size class.

### `SizeClass`

Manages a linked list of `Slab`s for a single slot size. On allocation, it walks its slabs looking for a free slot. If all slabs are full, it allocates a new 4 KB slab from the system and prepends it to the list.

### `Slab`

A single 4 KB page divided into `slot_size`-byte slots. Maintains an intrusive free list:

- **alloc**: pop the head of the free list → O(1)
- **dealloc**: push the slot back onto the free list → O(1)
- **contains**: bounds-check to determine if a pointer belongs to this slab

### `FreeNode`

A zero-cost abstraction — each free slot is reinterpreted as a `FreeNode` containing a single `next` pointer. Since the minimum slot size is 8 bytes, there is always room for a pointer.

## Configuration

Key constants at the top of `main.rs`:

| Constant | Default | Description |
|----------|---------|-------------|
| `SLAB_SIZE` | `4096` | Backing memory per slab (one typical page) |
| `SIZE_CLASSES` | `[8, 16, 32, 64, 128, 256, 512, 1024]` | Supported slot sizes in bytes |

Requests are rounded up to the nearest size class. Anything exceeding the largest class falls through to the system allocator.

## Design Tradeoffs

| Property | This Allocator | Notes |
|----------|---------------|-------|
| Allocation speed | O(1) | Free list pop |
| Deallocation speed | O(n) slabs per class | Linear scan to find owning slab |
| Internal fragmentation | Up to 2× | Rounding up to next size class |
| External fragmentation | None within a class | Slots are uniform |
| Thread safety | Atomic spin lock | Re-entrancy safe with system fallback |
| Large allocations | System fallback | Not slab-managed |

## Potential Enhancements

- **Slab reclamation** — return fully-empty slabs to the system to reduce memory footprint
- **Per-CPU / per-thread caches** — eliminate lock contention (à la Linux SLUB)
- **`no_std` support** — replace the system fallback with a custom page allocator
- **Allocation statistics** — track per-class usage, peak counts, and fragmentation ratios
- **Slab coloring** — offset slot starts to reduce cache line conflicts ([Bonwick, 1994](https://www.usenix.org/legacy/publications/library/proceedings/bos94/full_papers/bonwick.ps))
- **HashMap-based dealloc** — O(1) pointer → slab lookup instead of linear scan
- **`Allocator` trait (nightly)** — support per-collection allocators via `#![feature(allocator_api)]`

## Further Reading

- [The Slab Allocator: An Object-Caching Kernel Memory Allocator](https://www.usenix.org/legacy/publications/library/proceedings/bos94/full_papers/bonwick.ps) — Jeff Bonwick's original 1994 paper
- [SLUB: The Unqueued Slab Allocator](https://lwn.net/Articles/229984/) — Linux's modern slab implementation
- [The Rust `GlobalAlloc` trait](https://doc.rust-lang.org/std/alloc/trait.GlobalAlloc.html) — official documentation
- [Writing an OS in Rust: Allocator Designs](https://os.phil-opp.com/allocator-designs/) — Philipp Oppermann's excellent walkthrough

## License

MIT
