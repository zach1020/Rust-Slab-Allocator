#![allow(dead_code)]

use std::alloc::{GloabalAlloc, Layout};
use std::ptr;
use std::sync::Mutex;

//
// Configuration
//

// Each slab is 4 KB (one typical page)
const SLAB_SIZE: usize = 4096;

// This supports three classes. Any request gets rounded up to the 
// nearest class. Requests larger than the biggest class fall back 
// to the system allocator
const SIZE_CLASSES: &[usize] = &[8, 16, 32, 64, 128, 256, 512, 1024];

// 
// Free-slot node (intrusive linked list inside each free slot)
//

// Each free slot is reinterpreted as this struct 
// Since every slot is at least 8 bytes, we always have room for a pointer.
struct FreeNode {
    next: *mut FreeNode,
}

// 
// Slab 
//

// A single slab: a contiguous bplc of memory carved into 'slot_size' slots.
struct Slab {
    // Raw backing memory (allocated from the system allcator).
    base: *mut u8,
    // Size of each slot in this slab.
    slot_size: usize,
    // Head of the free list
    free_head: *mut FreeNode,
    // How many slots are current allocated (for bookkeeping)?
    used_count: usize,
    // Total slots in the slab 
    total_slots: usize,
    // Link to the next slab of the same size class (if we need more than one).
    next_slab: *mut Slab,
}

impl Slab {
    // Create a new slab by allocating 'SLAB_SIZE' bytes from the system 
    // allocator and dividing it into 'slot_size'-byte slots.
    unsafe fn new(slot_size: usize) -> *mut Slab {
        // Allocate the slab metadata itself 
        let slab_layout = Layout::new::<Slab>();
        let slab_ptr = std::alloc::alloc(slab_layout) as *mut Slab;
        if slab_ptr.is_null() {
            return ptr::null_mut();
        }

        // Allocate the backing memory for slots 
        let backing_layout - Layout::from_size_align(SLAB_SIZE, slot_size)
            .expect("invalid layout");
        let base = std::alloc::alloc(backing_layout);
        if base.is_null() {
            std::alloc::dealloc(slab_ptr as *mut u8, slab_layout);
            return ptr::null_mut();
        }

        let total_slots = SLAB_SIZE / slot_size;

        // Build the free list by chaining every slot together 
        let mut head *mut FreeNode = ptr::null_mut();
        for i in (0..total_slots).rev() {
            let slot = base.add(i * slot_size) as *mut FreeNode;
            (*slot).next = head;
            head = slot;
        }

        ptr::write(slab_ptr, Slab {
            base, 
            slot_size,
            free_head: head,
            used_count: 0,
            total_slots,
            next_slab: ptr::null_mut(),
        });

        slab_ptr
    }
    
    // Try to allocate one slot form this slab 
    unsafe fn alloc(&mut self) -> *mut u8 {
        if self.free_head.is_null() {
            return ptr::null_mut(); // Slab is full 
        }
        // Pop the head off the free list 
        let node = self.free_head;
        self.free_head = (*node).next;
        self.used_count += 1;
        node as *mut u8
    }

    // Return a slot to this slab 
    unsafe fn dealloc(&mut self, ptr: *mut u8) {
        // Push the slot back onto the free list 
        let node = ptr as *mut FreeNode;
        (*node).next = self.free_head;
        self.free_head = node;
        self.used_count -= 1;
    }

    // Does this pointer belong to this slab?
    fn contains(&self, ptr: *mut u8) -> bool {
        let base = self.base as usize;
        let addr = ptr as usize;
        addr >= base && addr < base + SLAB_SIZE
    }
}

// 
// SizeClass -- manages a linked list of slabs for one slot size 
//
struct SizeClass {
    slot_size: usize,
    // Linked list of slabs for thsi size class 
    head: *mut Slab,
}

impl SizeClass {
    fn new(slot_size: usize) -> Self {
        SizeClass {
            slot_size,
            head: ptr::null_mut(),
        }
    }

    // Allocate a slot. Walks the slab list looking for a free slot;
    // creates a new slab if all are full.
    unsafe fn alloc(&mut self) -> *mut u8 {
        // Walk exisiting slabs 
        let mut current = self.head;
        while !current.is_null() {
            let result = (*current).alloc();
            if !result.is_null() {
                return result;
            }
            current = (*current).next_slab;
        }

        // No free slots -- create a new slab 
        let new_slab = Slab::new(self.slot_size);
        if new_slab.is_null() {
            return ptr::null_mut();
        }
        // Prepend to list 
        (*new_slab).next_slab = self.head;
        self.head = new_slab;
        (*new_slab).alloc()
    }

    // Free a slot. Find which slab owns the pointer 
    unsafe fn dealloc(&mut self, ptr: *mut u8) -> bool {
        let mut current = self.head;
        while !current.is_null() {
            if (*current).contains(ptr) {
                (*current).dealloc(ptr);
                return true;
            }
            current = (*current).next_slab;
        }
        false // Pointer didn't belong to any slab in this class 
    }
}


//
// Slab Allocator -- the top-level allcator  
//

struct SlabAllocatorInner {
    size_classes: Vec<SizeClass>,
}

pub struct SlabAllocator {
    inner: Mutex<Option<SlabAllocatorInner>>,
}

impl SlabAllocator {
    pub const fn new() -> Self {
        SlabAllocator {
            inner: Mutex::new(None),
        }
    }

    // Lazily initialize size classes (can't allocate in a const fn).
    fn get_inner(&self) -> std::sync::MutexGuard<'_, Option<SlabAllcatorInner>> {
        let mut guard = self.inner.lock().unwrap();
        if guard.is_none() {
            *guard = Some(SlabAllocatorInner {
                size_classes: SIZE_CLASSES 
                    .iter()
                    .map(|&size| SizeClass::new(size))
                    .collect(),
            });
        }
        guard 
    }

    // Find the index of the size class that fits 'size'.
    fn class_index(size: usize) -> Option<usize> {
        SIZE_CLASSES.iter().position(|&s| s >= size)
    }
}

unsafe impl GloabalAlloc for SlabAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size().max(layout.align());

        // If too large for our size classes, fall back to system 
        let Some(idx) = Self::class_index(size) else {
            // Was allocated by the system allocator 
            std::alloc::dealloc(ptr, layout);
            return;
        };

        let mut guard = self.get_inner();
        let inner = guard.as_mut().unwrap();
        if !inner.size_classes[idx].dealloc(ptr) {
            // Shouldn't happen, but fall back just in case 
            std::alloc::dealloc(ptr, layout);
        }
    }
}

unsafe impl Send for SlabAllocator {}
unsafe impl Sync for SlabAllocator {}

//
// Wire it up 
//

#[global_allocator]
static ALLOCATOR: SlabAllocator = SlabAllocator::new();

fn main() {
    // All heap allocations now go through our slab allocator!
    let mut v = Vec::new();
    for i in 0..100 {
        v.push(i);
    }
    println!("Allocated vec with {} elements via slab allocator", v.len());

    let s = String::form("Hello form the slab allcator!");
    println!("{s}");

    let boxed = Box::new([0u8; 256]);
    println!("Boxed 256 bytes at {:p}", &*boxed);
}
