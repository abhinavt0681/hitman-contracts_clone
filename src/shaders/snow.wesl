// Blizzard streaks: every flake is a quad whose position is computed here from a seed, the
// wind's accumulated travel and the camera, so the CPU never touches them. Flakes under a
// surface of the level's top-down height map are dropped, which keeps snow out of buildings.

import bevy_pbr::render::mesh_view_bindings::view;

struct Snow {
    // xyz: how far the wind has carried the air so far (m); w: seconds since start.
    travel: vec4<f32>,
    // xyz: wind velocity now (m/s); w: streak exposure time (s).
    wind: vec4<f32>,
    // xyz: half size of the box of flakes around the camera; w: share of flakes shown.
    half_box: vec4<f32>,
    // rgb: fog colour; w: fog density.
    fog: vec4<f32>,
    // rgb: flake colour in ambient light; w: opacity.
    tint: vec4<f32>,
    // xy: height map origin (world x, z); z: cell size (m).
    map: vec4<f32>,
    // x: lamps in use.
    lamps: vec4<f32>,
    // xyz: position; w: reach.
    lamp_pos: array<vec4<f32>, 16>,
    lamp_col: array<vec4<f32>, 16>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> snow: Snow;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var height_map: texture_2d<f32>;

struct Vertex {
    // Position in the unit box.
    @location(0) seed: vec3<f32>,
    // Size, speed and flutter phase, each 0..1.
    @location(1) rand: vec3<f32>,
    // Quad corner, 0..1.
    @location(2) corner: vec2<f32>,
}

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) color: vec4<f32>,
}

// Height of the highest surface above `p`'s column, or far below when off the map.
fn top_at(p: vec3<f32>) -> f32 {
    let dims = vec2<i32>(textureDimensions(height_map));
    let c = vec2<i32>(floor((p.xz - snow.map.xy) / snow.map.z));
    if any(c < vec2<i32>(0)) || any(c >= dims) {
        return -1e9;
    }
    return textureLoad(height_map, c, 0).r;
}

fn lamp_light(p: vec3<f32>) -> vec3<f32> {
    var light = vec3<f32>(0.0);
    let n = u32(snow.lamps.x);
    for (var i = 0u; i < n; i++) {
        let l = snow.lamp_pos[i];
        let f = clamp(1.0 - distance(l.xyz, p) / l.w, 0.0, 1.0);
        light += snow.lamp_col[i].rgb * f * f;
    }
    return light;
}

@vertex
fn vertex(v: Vertex) -> Out {
    var out: Out;
    out.corner = v.corner;
    let cam = view.world_position;
    let size = snow.half_box.xyz * 2.0;
    let speed = mix(0.75, 1.25, v.rand.y);
    // Each flake rides the wind at its own pace, wrapped into a box that follows the camera.
    let local = fract((v.seed * size + snow.travel.xyz * speed - cam) / size) * size - snow.half_box.xyz;
    let t = snow.travel.w;
    let phase = v.rand.z * 6.2831;
    let p = cam + local + vec3(sin(t * 1.7 + phase), cos(t * 2.3 + phase * 1.3), sin(t * 1.3 + phase * 0.7)) * 0.12;

    if v.rand.x > snow.half_box.w || p.y < top_at(p) + 0.02 {
        out.clip = vec4(0.0, 0.0, -2.0, 1.0);
        out.color = vec4(0.0);
        return out;
    }

    // A streak: the distance the flake travels while the eye's "shutter" is open.
    let vel = snow.wind.xyz * speed;
    let along = normalize(vel);
    let dist = distance(cam, p);
    var side = cross(along, (cam - p) / max(dist, 1e-3));
    if length(side) < 1e-3 {
        side = vec3(0.0, 1.0, 0.0);
    }
    side = normalize(side);
    // Seen end-on a streak would fan out from a vanishing point; keep those short.
    let end_on = abs(dot(along, (p - cam) / max(dist, 1e-3)));
    let len = (length(vel) * snow.wind.w * mix(0.6, 1.4, v.rand.x) + 0.03) * (1.0 - 0.7 * end_on * end_on);
    // Wider with distance so far streaks don't vanish between pixels.
    let width = mix(0.005, 0.014, v.rand.x) + dist * 0.0009;
    let q = p + along * (v.corner.y - 0.5) * len + side * (v.corner.x - 0.5) * width;
    out.clip = view.clip_from_world * vec4(q, 1.0);

    let light = snow.tint.rgb + lamp_light(p);
    let near = smoothstep(0.6, 2.5, dist);
    let far = 1.0 - smoothstep(snow.half_box.x * 0.55, snow.half_box.x, length(local.xz));
    let fog = exp(-snow.fog.w * dist);
    out.color = vec4(mix(snow.fog.rgb, light, fog), snow.tint.w * near * far * mix(0.45, 1.0, v.rand.z));
    return out;
}

@fragment
fn fragment(in: Out) -> @location(0) vec4<f32> {
    let across = 1.0 - abs(in.corner.x * 2.0 - 1.0);
    let along = sin(in.corner.y * 3.14159);
    return vec4(in.color.rgb, in.color.a * across * along);
}
