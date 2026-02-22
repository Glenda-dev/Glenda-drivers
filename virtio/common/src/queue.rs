#[repr(C, align(16))]
#[derive(Debug, Clone, Copy)]
pub struct Descriptor {
    pub addr: u64,
    pub len: u32,
    pub flags: u16,
    pub next: u16,
}

pub const DESC_F_NEXT: u16 = 1;
pub const DESC_F_WRITE: u16 = 2;
pub const DESC_F_INDIRECT: u16 = 4;

#[repr(C, align(2))]
pub struct Available {
    pub flags: u16,
    pub idx: u16,
    pub ring: [u16; 0], // Flexible array
}

#[repr(C, align(4))]
pub struct UsedElem {
    pub id: u32,
    pub len: u32,
}

#[repr(C, align(4))]
pub struct Used {
    pub flags: u16,
    pub idx: u16,
    pub ring: [UsedElem; 0], // Flexible array
}

// Helper to calculate queue size
pub fn queue_size_in_bytes(num: u16) -> usize {
    let num = num as usize;
    let desc_size = 16 * num;
    let avail_size = 6 + 2 * num;
    let used_size = 6 + 8 * num;

    // Page align each section or just return total?
    // VirtIO 1.0 allows separate addresses for each part.
    desc_size + avail_size + used_size
}

pub struct VirtQueue {
    pub index: u32,
    pub num: u16,
    pub paddr: u64,
    pub vaddr: *mut u8,

    pub last_used_idx: u16,
    pub free_head: u16,
    pub num_free: u16,
}

impl VirtQueue {
    pub unsafe fn new(index: u32, num: u16, paddr: u64, vaddr: *mut u8) -> Self {
        let v = Self { index, num, paddr, vaddr, last_used_idx: 0, free_head: 0, num_free: num };

        // Initialize descriptor chain
        let descs = v.desc_table();
        for i in 0..num - 1 {
            descs[i as usize].next = i + 1;
            descs[i as usize].flags = DESC_F_NEXT;
        }
        descs[(num - 1) as usize].next = 0;
        descs[(num - 1) as usize].flags = 0;

        v
    }

    pub fn desc_table(&self) -> &mut [Descriptor] {
        unsafe { core::slice::from_raw_parts_mut(self.vaddr as *mut Descriptor, self.num as usize) }
    }

    pub fn avail_ring(&self) -> &mut Available {
        unsafe {
            let offset = 16 * self.num as usize;
            &mut *(self.vaddr.add(offset) as *mut Available)
        }
    }

    pub fn used_ring(&self) -> &mut Used {
        unsafe {
            // Align to 4 bytes for Used ring features
            let offset = (16 * self.num as usize + 6 + 2 * self.num as usize + 3) & !3;
            &mut *(self.vaddr.add(offset) as *mut Used)
        }
    }

    pub fn alloc_desc(&mut self) -> Option<u16> {
        if self.num_free == 0 {
            return None;
        }
        let id = self.free_head;
        unsafe {
            let next_ptr = &self.desc_table()[id as usize].next as *const u16;
            self.free_head = next_ptr.read_volatile();
        }
        self.num_free -= 1;
        Some(id)
    }

    pub fn free_desc(&mut self, id: u16) {
        unsafe {
            let next_ptr = &mut self.desc_table()[id as usize].next as *mut u16;
            let flags_ptr = &mut self.desc_table()[id as usize].flags as *mut u16;
            next_ptr.write_volatile(self.free_head);
            flags_ptr.write_volatile(DESC_F_NEXT);
        }
        self.free_head = id;
        self.num_free += 1;
    }

    pub fn write_desc(&mut self, id: u16, desc: Descriptor) {
        unsafe {
            let ptr = self.desc_table().as_mut_ptr().add(id as usize);
            ptr.write_volatile(desc);
            core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        }
    }

    pub fn submit(&mut self, head: u16) {
        let avail = self.avail_ring();
        unsafe {
            // Must use volatile read/write for idx
            let idx_ptr = core::ptr::addr_of_mut!(avail.idx);
            let idx = idx_ptr.read_volatile();
            let ring_idx = idx as usize % self.num as usize;

            let ring_ptr = self.vaddr.add(16 * self.num as usize + 4) as *mut u16;
            ring_ptr.add(ring_idx).write_volatile(head);
            glenda::arch::sync::fence();

            idx_ptr.write_volatile(idx.wrapping_add(1));
        }
    }

    pub fn can_pop(&self) -> bool {
        let used = self.used_ring();
        unsafe {
            let idx_ptr = &used.idx as *const u16;
            self.last_used_idx != idx_ptr.read_volatile()
        }
    }

    pub fn pop(&mut self) -> Option<(u32, u32)> {
        if !self.can_pop() {
            return None;
        }
        // Memory barrier to ensure Used ring updates are visible after readingUsed.idx
        glenda::arch::sync::fence();

        let used = self.used_ring();
        let ring_idx = self.last_used_idx as usize % self.num as usize;
        let elem = unsafe {
            let ring_ptr = (used as *const Used).add(1) as *const UsedElem;
            ring_ptr.add(ring_idx).read_volatile()
        };
        self.last_used_idx = self.last_used_idx.wrapping_add(1);
        Some((elem.id, elem.len))
    }
}
