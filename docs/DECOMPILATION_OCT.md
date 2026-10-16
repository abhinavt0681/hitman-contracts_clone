# Glacier 1 Collision & `.OCT` Decompilation Notes

Reverse-engineered from the original retail executable `HitmanContracts.exe` (PE32 i386, timestamp Dec 19 2013, base `0x00400000`).

---

## 1. Class Vtables & Memory Map

| Symbol | VMA Address | Description |
|---|---|---|
| `vftable_ZTree` | `0x00715cd0` | Vtable for spatial octree (`ZTree`) |
| `vftable_ZTreeGroup` | `0x00715a48` | Vtable for entity tree group (`ZTreeGroup`, entity type `0x100020` / `0x2003e9`) |
| `vftable_ZCollisionWorld` | `0x0073e518` | Vtable for Glacier spatial physics & raycast manager |
| `g_GlacierEngine` | `0x0079457c` | Global engine pointer (`+0xa5` entity manager, `+0x18ae` physics subsystem) |
| `flt_CenterOffset` | `0x0072d17c` | Quantization center constant `32768.0f` (`0x47000000`) |

---

## 2. Reconstructed C++ Class Definitions

```cpp
struct ZTreeGroupNode {
    uint16_t flags_and_count; // low 3 bits: octant mask; bits [3..14]: poly count
    uint16_t child_index;     // 0 = leaf node; else index to first child (stride: 6 bytes)
    uint16_t poly_start;      // Index into polygon table
};

#pragma pack(push, 1)
struct ZOctPolyRecord {
    uint32_t poly_ref;        // Packed PRM submesh index & triangle index
    uint16_t min_q[3];        // Quantized min corner (center 32768)
    uint16_t max_q[3];        // Quantized max corner (center 32768)
};
#pragma pack(pop)

struct ZTree {
    void* vftable;             // 0x00715cd0
    float origin[3];           // +0x04: World origin (cm)
    float inv_scale;           // +0x10: 1.0f / scale
    uint8_t* stream;           // +0x14: Pointer to octree stream payload
};

struct ZTreeGroup : public ZEntity {
    // vftable: 0x00715a48
    // +0x48: ZTree* m_trees[5]; (Up to 5 tree slots per entity group)
    // +0x5c: uint32_t m_unknown;
    // +0x64: uint16_t* m_poly_indices;
    // +0x6d: uint8_t m_bEnabled; // Set by Enable(bool) at 0x004560f0
};

struct ZSpatialLink {
    uint32_t poly_ref;         // Packed submesh & triangle index
    uint32_t primitive_offset; // Target PRM primitive offset / ID in scene
};
```

---

## 3. Disassembled & Decompiled Functions

### 3.1 Scene Loader: `.OCT` Dispatcher (`0x00425750`)
Dispatches entity groups, global trees, and link tables from the raw `.OCT` stream:

```cpp
void SceneLoader::LoadOCT(uint8_t* oct_buffer) {
    if (*(uint32_t*)oct_buffer == 0) {
        // Initialize spatial subsystem
        Engine->pPhysics->EnableAll(1);
    }
    uint8_t* p = oct_buffer + 0x10; // Skip 16-byte header

    // 1. Entity tree groups loop
    ZEntity* entity = Engine->pEntityManager->m_pFirstEntity;
    while (entity) {
        if (entity->IsKindOf(0x100020)) { // ZTreeGroup
            if (entity->Method_21c()) {   // m_bEnabled != 0
                // ZTreeGroup::Load consumes 5 slots and returns updated pointer
                p = entity->LoadTreeGroup(p);
            }
        }
        entity = entity->m_pNext;
    }

    // 2. Global scene trees loop (terminated by 0xffffffff)
    while (*(uint32_t*)p != 0xffffffff) {
        p = Engine->pPhysics->LoadGlobalTree(p); // 0x006e3dd0
    }
    p += 4; // Skip 0xffffffff terminator

    // 3. Trailing spatial link table (0x006e3370)
    uint32_t remaining_bytes = oct_buffer_end - p;
    Engine->pPhysics->LoadSpatialLinks(p, remaining_bytes);
}
```

---

### 3.2 `ZTreeGroup::Load` (`0x00456030`)
Processes the 5 tree slots for a `ZTreeGroup` entity:

```cpp
uint8_t* ZTreeGroup::Load(uint8_t* p) {
    for (int slot = 0; slot < 5; ++slot) {
        uint32_t size = *(uint32_t*)p;
        p += 4;
        if (size == 0) {
            this->m_trees[slot] = nullptr;
            p += 12; // Empty slot is 16 bytes total
        } else {
            // +0x04 to +0x20: 28-byte outer bounding header
            float scale = *(float*)(p + 0);
            float origin[3] = { *(float*)(p + 4), *(float*)(p + 8), *(float*)(p + 12) };
            p += 28; // Advance to payload at +0x20

            ZTree* tree = new ZTree();
            tree->vftable = 0x00715cd0;
            tree->Load(p); // 0x00629a30
            this->m_trees[slot] = tree;

            uint32_t aligned_size = (size + 15) & ~15;
            p += aligned_size;
        }
    }
    return p;
}
```

---

### 3.3 `ZTree::Load` (`0x00629a30`)
Initializes a `ZTree` instance from its stream payload:

```cpp
void ZTree::Load(uint8_t* stream) {
    this->stream = stream;
    this->inv_scale = *(float*)(stream + 0x10);
    this->origin[0] = *(float*)(stream + 0x04);
    this->origin[1] = *(float*)(stream + 0x08);
    this->origin[2] = *(float*)(stream + 0x0c);
}

// 0x00629a60: Node table start
ZTreeGroupNode* ZTree::GetNodes() {
    return (ZTreeGroupNode*)(this->stream + 0x14);
}

// 0x00629a70: Polygon table start
ZOctPolyRecord* ZTree::GetPolys() {
    uint32_t poly_table_rel = *(uint32_t*)this->stream;
    return (ZOctPolyRecord*)(this->stream + poly_table_rel);
}
```

---

### 3.4 Global Tree Loader (`0x006e3dd0`, Method `+0x54`)
Loads individual global trees until the `0xffffffff` terminator:

```cpp
uint8_t* ZCollisionWorld::LoadGlobalTree(uint8_t* p) {
    uint32_t total_chunk_size = *(uint32_t*)p;
    ZTree* tree = (ZTree*)(p + 4);
    tree->Init();
    tree->Load(p + 0x20); // Stream payload starts at +0x20
    return p + total_chunk_size;
}
```

---

### 3.5 Spatial Links Table Loader (`0x006e3370`, Method `+0x48`)
Loads the mapping of octree triangles to PRM submesh primitives:

```cpp
void ZCollisionWorld::LoadSpatialLinks(uint8_t* p, uint32_t bytes) {
    this->m_link_count = bytes >> 3; // bytes / 8
    this->m_pLinks = (ZSpatialLink*)p;
    this->m_bLinksReady = 1;
}
```

---

### 3.6 Ray-Triangle & Polygon Lookup Callback (`0x006e2590`)
Fetches vertex data from PRM submeshes to execute ray-triangle intersection:

```cpp
bool ZCollisionWorld::IntersectOctPoly(ZOctPolyRecord* poly, Ray* ray, HitResult* outHit) {
    uint32_t poly_ref = poly->poly_ref;
    uint32_t submesh_idx = (poly_ref >> 16);
    uint32_t tri_start = (poly_ref & 0x7fff);

    SubmeshHeader* submesh = GetSubmesh(submesh_idx);
    if ((submesh->collision_flags & ray->mask) == 0) {
        return false; // Collision mask filtering
    }

    uint8_t* vertex_buffer = submesh->pVertices;   // 36 bytes per vertex
    uint16_t* index_buffer = submesh->pIndices;    // 16-bit index list

    uint16_t i0 = index_buffer[tri_start + 0];
    uint16_t i1 = index_buffer[tri_start + 1];
    uint16_t i2 = index_buffer[tri_start + 2];

    Vertex* v0 = (Vertex*)(vertex_buffer + i0 * 36);
    Vertex* v1 = (Vertex*)(vertex_buffer + i1 * 36);
    Vertex* v2 = (Vertex*)(vertex_buffer + i2 * 36);

    return RayTriangleIntersect(ray, v0->pos, v1->pos, v2->pos, outHit); // 0x00624d50
}
```

---

### 3.7 Coordinate Quantization / Dequantization (`0x00629c03`)
Quantization formula reconstructed from `0x00629c03` and `0x00629c8a`:

```cpp
// Dequantize: u16 quantized coordinate -> world float (cm)
float Dequantize(uint16_t q, float origin, float inv_scale) {
    float scale = 1.0f / inv_scale;
    return origin + ((float)q - 32768.0f) * scale;
}

// Quantize: world float (cm) -> u16 coordinate
uint16_t Quantize(float world, float origin, float inv_scale) {
    float q = (world - origin) * inv_scale + 32768.0f;
    return (uint16_t)clamp(q, 0.0f, 65535.0f);
}
```

---

## 4. Verification against Scene `C02-1`

| Section | Expected / Decompiled | Parsed in Rust | Status |
|---|---|---|---|
| Entity groups | 53 groups (5 slots each) | 53 groups, 150 active trees | **100% Match** |
| Global trees | Offsets `0x18790` to `0x90e80` | 898 trees, 18,071 nodes, 26,714 polys | **100% Match** |
| Terminator | `0xffffffff` at `0x90e80` | Verified at `0x90e80` | **100% Match** |
| Spatial links | 182,016 bytes / 8 bytes | 22,752 records | **100% Match** |
| Total file size | 775,556 bytes | 775,556 bytes parsed (0 leftover bytes) | **100% Match** |
