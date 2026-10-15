// Blowing snow: big upright sheets of noise that ride the wind, either billowing across the
// yard at eye height or skimming the ground as drifting powder. Same uniforms as the streaks.

import bevy_pbr::render::mesh_view_bindings::view;

struct Snow {
    // xyz: how far the wind has carried the air so far (m); w: seconds since start.
    travel: vec4<f32>,
    // xyz: wind velocity now (m/s); w: unused.
    wind: vec4<f32>,
    // x, z: half size of the area of sheets around the camera; y: sheet height (m);
    // w: 1 for powder hugging the ground, 0 for sheets at eye height.
    half_box: vec4<f32>,
    // rgb: fog colour; w: fog density.
    fog: vec4<f32>,
    // rgb: snow colour in ambient light; w: opacity.
    tint: vec4<f32>,
    // xy: height map origin (world x, z); z: cell size (m).
    map: vec4<f32>,
    // x: lamps in use.
    lamps: vec4<f32>,
    lamp_pos: array<vec4<f32>, 16>,
    lamp_col: array<vec4<f32>, 16>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> snow: Snow;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var height_map: texture_2d<f32>;

struct Vertex {
    @location(0) seed: vec3<f32>,
    // Width, height and pace, each 0..1.
    @location(1) rand: vec3<f32>,
    @location(2) corner: vec2<f32>,
}

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) world: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) pace: f32,
}

fn top_at(p: vec2<f32>) -> f32 {
    let dims = vec2<i32>(textureDimensions(height_map));
    let c = vec2<i32>(floor((p - snow.map.xy) / snow.map.z));
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
        let f = clamp(1.0 - distance(l.xyz, p) / (l.w * 1.3), 0.0, 1.0);
        light += snow.lamp_col[i].rgb * f * f;
    }
    return light;
}

@vertex
fn vertex(v: Vertex) -> Out {
    var out: Out;
    out.corner = v.corner;
    let cam = view.world_position;
    let ground_mode = snow.half_box.w > 0.5;
    let area = snow.half_box.xz * 2.0;
    let pace = mix(0.85, 1.15, v.rand.z);
    out.pace = pace;
    let local = fract((v.seed.xz * area + snow.travel.xz * pace - cam.xz) / area) * area - snow.half_box.xz;
    let centre = cam.xz + local;
    let top = top_at(centre);
    var base: f32;
    var hidden: bool;
    if ground_mode {
        base = top - 0.1;
        hidden = top < cam.y - 8.0 || top > cam.y + 2.0;
    } else {
        base = max(top, cam.y - 3.0) - 0.5;
        hidden = top > cam.y + 1.0;
    }
    let width = select(mix(10.0, 24.0, v.rand.x), mix(5.0, 12.0, v.rand.x), ground_mode);
    let height = snow.half_box.y * mix(0.6, 1.3, v.rand.y);
    let to_cam = cam.xz - centre;
    let right2 = normalize(vec2(-to_cam.y, to_cam.x) + vec2(1e-4, 0.0));
    let p = vec3(centre.x, base, centre.y) + vec3(right2.x, 0.0, right2.y) * (v.corner.x - 0.5) * width + vec3(0.0, v.corner.y * height, 0.0);
    out.world = p;
    if hidden {
        out.clip = vec4(0.0, 0.0, -2.0, 1.0);
        out.color = vec4(0.0);
        return out;
    }
    out.clip = view.clip_from_world * vec4(p, 1.0);
    let dist = distance(cam, p);
    let fog = exp(-snow.fog.w * dist * 0.8);
    let light = snow.tint.rgb + lamp_light(p);
    // Fade out close up (no walls of noise in the face) and at the edge of the area.
    let near = smoothstep(2.0, 9.0, length(to_cam));
    let far = 1.0 - smoothstep(snow.half_box.x * 0.6, snow.half_box.x, length(local));
    out.color = vec4(mix(snow.fog.rgb, light, fog), snow.tint.w * near * far);
    return out;
}

fn hash3(p: vec3<f32>) -> f32 {
    let q = fract(p * 0.3183099 + vec3(0.71, 0.113, 0.419));
    let r = q * 17.0;
    return fract(r.x * r.y * r.z * (r.x + r.y + r.z));
}

fn noise3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(mix(hash3(i), hash3(i + vec3(1.0, 0.0, 0.0)), u.x), mix(hash3(i + vec3(0.0, 1.0, 0.0)), hash3(i + vec3(1.0, 1.0, 0.0)), u.x), u.y),
        mix(mix(hash3(i + vec3(0.0, 0.0, 1.0)), hash3(i + vec3(1.0, 0.0, 1.0)), u.x), mix(hash3(i + vec3(0.0, 1.0, 1.0)), hash3(i + vec3(1.0, 1.0, 1.0)), u.x), u.y),
        u.z,
    );
}

fn fbm(p: vec3<f32>) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var q = p;
    for (var i = 0; i < 4; i++) {
        sum += amp * noise3(q);
        q = q * 2.03 + vec3(1.7, 9.2, 3.1);
        amp *= 0.5;
    }
    return sum;
}

@fragment
fn fragment(in: Out) -> @location(0) vec4<f32> {
    let ground_mode = snow.half_box.w > 0.5;
    // Noise that moves with the air, stretched along the ground and churning as it goes.
    let air = in.world - snow.travel.xyz * in.pace;
    let scale = select(vec3(0.09, 0.22, 0.09), vec3(0.16, 0.9, 0.16), ground_mode);
    let n = fbm(air * scale + vec3(0.0, snow.travel.w * 0.15, 0.0));
    let density = smoothstep(0.38, 0.75, n);
    let across = sin(in.corner.x * 3.14159);
    var updown: f32;
    if ground_mode {
        updown = 1.0 - smoothstep(0.15, 1.0, in.corner.y);
    } else {
        updown = smoothstep(0.0, 0.25, in.corner.y) * (1.0 - smoothstep(0.5, 1.0, in.corner.y));
    }
    return vec4(in.color.rgb, in.color.a * density * across * updown);
}
