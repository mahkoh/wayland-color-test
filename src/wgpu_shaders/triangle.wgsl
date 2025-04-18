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

struct Data {
    r: vec2f,
    g: vec2f,
    b: vec2f,
    wp: vec2f,
};

var<push_constant> data: Data;

@fragment
fn triangle_frag_main(in: FragIn) -> @location(0) vec4f {
    let xy = in.pos * vec2f(0.85, 0.85);

    const TRIANGLE_WHITE = 0.007;
    const TRIANGLE_BLACK = 0.005;
    const WP_WHITE = 0.015;
    const WP_BLACK = TRIANGLE_BLACK + (WP_WHITE - TRIANGLE_WHITE);

    var triangle_alpha: f32;
    {
        let dist = distance_to_triangle_edge(xy, data.r, data.g, data.b);
        triangle_alpha = 1.0 - smoothstep(TRIANGLE_BLACK, TRIANGLE_WHITE, dist);
    }
    var wp_alpha = 1.0 - smoothstep(WP_BLACK, WP_WHITE, length(xy - data.wp));

    let alpha = max(triangle_alpha, wp_alpha);
    return vec4f(0.0, 0.0, 0.0, alpha);
}

fn distance_to_triangle_edge(p: vec2f, p0: vec2f, p1: vec2f, p2: vec2f) -> f32 {
    let d0 = distance_to_line_squared(p, p0, p1);
    let d1 = distance_to_line_squared(p, p1, p2);
    let d2 = distance_to_line_squared(p, p2, p0);

    let d = min(min(d0, d1), d2);

    return sqrt(d);
}

fn distance_to_line_squared(p: vec2f, p0: vec2f, p1: vec2f) -> f32 {
    let e = p1 - p0;
    let v = p - p0;
    let d = v - e * clamp(dot(v, e) / dot(e, e), 0.0, 1.0);
    return dot(d, d);
}
