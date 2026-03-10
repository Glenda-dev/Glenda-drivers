//! VirtIO-GPU Protocol definitions
//! References: VirtIO Spec 1.1, Section 5.7

/// VirtIO-GPU Control Command Types
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuCmdType {
    GetDisplayInfo = 0x0100,
    ResourceCreate2d = 0x0101,
    ResourceUnref = 0x0102,
    SetScanout = 0x0103,
    ResourceFlush = 0x0104,
    TransferToHost2d = 0x0105,
    ResourceAttachBacking = 0x0106,
    ResourceDetachBacking = 0x0107,
    CtxCreate = 0x0108,
    CtxDestroy = 0x0109,
    CtxAttachResource = 0x010a,
    CtxDetachResource = 0x010b,
    GetCapsetInfo = 0x010c,
    GetCapset = 0x010d,
    GetEdid = 0x010e,

    /* 3d commands */
    ResourceCreate3d = 0x0200,
    AttachBacking3d = 0x0201,
    SetScanoutBlob = 0x0202,

    /* cursor commands */
    UpdateCursor = 0x0300,
    MoveCursor = 0x0301,

    /* success responses */
    RespOkNoData = 0x1100,
    RespOkDisplayInfo = 0x1101,
    RespOkCapsetInfo = 0x1102,
    RespOkCapset = 0x1103,
    RespOkEdid = 0x1104,

    /* error responses */
    RespErrUnspec = 0x1200,
    RespErrOutOfMemory = 0x1201,
    RespErrInvalidScanout = 0x1202,
    RespErrInvalidResourceId = 0x1203,
    RespErrInvalidContextId = 0x1204,
    RespErrInvalidParameter = 0x1205,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct GpuHeader {
    pub ty: u32,
    pub flags: u32,
    pub fence_id: u64,
    pub ctx_id: u32,
    pub padding: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct GpuRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuFormats {
    B8G8R8A8Unorm = 1,
    B8G8R8X8Unorm = 2,
    A8R8G8B8Unorm = 3,
    X8R8G8B8Unorm = 4,
    R8G8B8A8Unorm = 67,
    X8B8G8R8Unorm = 68,
    A8B8G8R8Unorm = 121,
    R8G8B8X8Unorm = 134,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct GpuResourceCreate2d {
    pub hdr: GpuHeader,
    pub resource_id: u32,
    pub format: u32,
    pub width: u32,
    pub height: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct GpuResourceAttachBacking {
    pub hdr: GpuHeader,
    pub resource_id: u32,
    pub nr_entries: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct GpuMemEntry {
    pub addr: u64,
    pub length: u32,
    pub padding: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct GpuSetScanout {
    pub hdr: GpuHeader,
    pub r: GpuRect,
    pub scanout_id: u32,
    pub resource_id: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct GpuTransferToHost2d {
    pub hdr: GpuHeader,
    pub r: GpuRect,
    pub offset: u64,
    pub resource_id: u32,
    pub padding: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct GpuResourceFlush {
    pub hdr: GpuHeader,
    pub r: GpuRect,
    pub resource_id: u32,
    pub padding: u32,
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct GpuDisplayInfo {
    pub hdr: GpuHeader,
    pub pmodes: [GpuDisplayMode; 16],
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct GpuDisplayMode {
    pub r: GpuRect,
    pub enabled: u32,
    pub flags: u32,
}
