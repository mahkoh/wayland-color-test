struct VtxOut {
    @builtin(position) pos: vec4f,
    @location(0) pos2: vec2f,
}

@vertex
fn vtx_main(@builtin(vertex_index) vertex_index : u32) -> VtxOut {
    const pos = array(
        vec2( 1.0, -1.0),
        vec2(-1.0, -1.0),
        vec2( 1.0,  1.0),
        vec2(-1.0,  1.0),
    );
    let pos2 = pos[vertex_index];
    var out = VtxOut();
    out.pos2 = (pos2 + vec2f(1.0)) / vec2f(2.0);
    out.pos = vec4f(pos2, 0, 1);
    return out;
}

struct FragIn {
    @location(0) pos: vec2f,
}

@fragment
fn frag_main(in: FragIn) -> @location(0) vec4f {
    let xy = in.pos * vec2f(0.85, 0.85);

    const Y = 1.0;
    let y_ratio = Y / xy.y;
    let XYZ = vec3f(xy.x * y_ratio, Y, (1.0 - xy.x - xy.y) * y_ratio);

    var col = XYZtoSRGB * XYZ;
    col = normalize(col);

    let distance = distance_to_horseshoe(xy);
    col = mix(col, vec3f(1.0), smoothstep(0.005, 0.007, distance));

    return vec4f(col, 1.0);
}

fn left_of(p: vec2f, a: vec2f, b: vec2f) -> bool {
    let u = b - a;
    let v = p - a;
    return (u.x * v.y - u.y * v.x) < 0.0;
}

fn g(x: f32, peak: f32, falloff_left: f32, falloff_right: f32) -> f32 {
    var value = x - peak;
    value = -0.5 * (value * value);
    if (x < peak) {
        return exp(falloff_left * falloff_left * value);
    } else {
        return exp(falloff_right * falloff_right * value);
    }
}

fn wavelength_to_xy(wavelength: f32) -> vec2f {
    let XYZ = vec3f(
        1.056 * g(wavelength, 599.8, 0.0264, 0.0323) + 0.362 * g(wavelength, 442.0, 0.0624, 0.0374) - 0.065 * g(wavelength, 501.1, 0.0490, 0.0382),
        0.821 * g(wavelength, 568.8, 0.0213, 0.0247) + 0.286 * g(wavelength, 530.9, 0.0613, 0.0322),
        1.217 * g(wavelength, 437.0, 0.0845, 0.0278) + 0.681 * g(wavelength, 459.0, 0.0385, 0.0725),
    );
    return XYZ.xy/(XYZ.x + XYZ.y + XYZ.z);
}

fn distance_to_horseshoe(uv: vec2f) -> f32 {
    var outside = false;
    var distance = 2.0;
    const MIN_WL = 440.0;
    const MAX_WL = 646.0;
    const STEP = 3.0;
    var prev_xy = wavelength_to_xy(MIN_WL);

    for (var wavelength = MIN_WL + STEP; wavelength < MAX_WL; wavelength += STEP) {
        let xy = wavelength_to_xy(wavelength);
        outside = outside || left_of(uv, xy, prev_xy);
        distance = min(distance, distance_to_line_squared(uv, xy, prev_xy));
        prev_xy = xy;
    }
    let xy = wavelength_to_xy(MIN_WL);
    outside = outside || left_of(uv, xy, prev_xy);
    distance = min(distance, distance_to_line_squared(uv, xy, prev_xy));

    return f32(outside) * sqrt(distance);
}

fn distance_to_line_squared(p: vec2f, p0: vec2f, p1: vec2f) -> f32 {
    let e = p1 - p0;
    let v = p - p0;
    let d = v - e * clamp(dot(v, e) / dot(e, e), 0.0, 1.0);
    return dot(d, d);
}

const XYZtoSRGB = mat3x3(
    3.2406, -0.9689,  0.0557,
   -1.5372,  1.8758, -0.2040,
   -0.4986,  0.0415,  1.0570,
);
