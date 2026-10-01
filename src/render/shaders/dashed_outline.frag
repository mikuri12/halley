precision highp float;
//_DEFINES

varying vec2 v_coords;
uniform sampler2D tex;
uniform float alpha;
uniform vec4 outline_color;
uniform vec2 rect_size;
uniform float thickness;
uniform float dash_period;
uniform float dash_length;

void main() {
    vec2 size = max(rect_size, vec2(1.0));
    vec2 p = v_coords * size;

    // Distance to each edge, and which edge this fragment belongs to.
    float dl = p.x;
    float dr = size.x - p.x;
    float dt = p.y;
    float db = size.y - p.y;
    float edge = min(min(dl, dr), min(dt, db));

    float width = max(thickness, 1.0);
    float band = 1.0 - smoothstep(width - 0.75, width + 0.75, edge);
    if (band <= 0.0) { discard; }

    // Unroll the outline into a single perimeter coordinate so dashes keep a
    // constant pitch all the way around instead of restarting per edge.
    float s;
    if (dt <= edge) {
        s = p.x;
    } else if (dr <= edge) {
        s = size.x + p.y;
    } else if (db <= edge) {
        s = size.x + size.y + (size.x - p.x);
    } else {
        s = 2.0 * size.x + size.y + (size.y - p.y);
    }

    // Distribute any remainder across the dashes so the pattern closes cleanly
    // at the starting corner rather than leaving a stub.
    float perimeter = 2.0 * (size.x + size.y);
    float count = max(floor(perimeter / max(dash_period, 1.0)), 1.0);
    float period = perimeter / count;
    float on = clamp(dash_length, 1.0, period);
    float phase = mod(s, period);
    float dash = (1.0 - smoothstep(on - 0.75, on + 0.75, phase))
        * smoothstep(-0.75, 0.75, phase);

    float coverage = band * dash;
    if (coverage <= 0.0) { discard; }
    gl_FragColor = outline_color * (coverage * alpha);
}
