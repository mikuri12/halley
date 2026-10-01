precision highp float;
//_DEFINES

varying vec2 v_coords;
uniform sampler2D tex;
uniform float alpha;
uniform vec2 rect_size;
uniform vec2 caster_size;
uniform vec2 caster_center;
uniform vec2 hole_center;
uniform vec2 corner_radii;
uniform float spread;
uniform float shadow_radius;
uniform vec4 shadow_color;

float rounded_rect_sdf(vec2 p, vec2 size, vec2 radii) {
    float radius = p.y < 0.0 ? radii.x : radii.y;
    vec2 half_size = size * 0.5;
    vec2 q = abs(p) - (half_size - vec2(radius));
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
}

float erf_approx(float x) {
    float s = sign(x);
    float a = abs(x);
    float t = 1.0 / (1.0 + 0.3275911 * a);
    float y = 1.0 - (((((1.061405429 * t - 1.453152027) * t)
        + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * exp(-a * a);
    return s * y;
}

void main() {
    vec2 size = max(rect_size, vec2(1.0));
    vec2 caster = max(caster_size, vec2(1.0));
    vec2 p = v_coords * size - caster_center;
    vec2 radii = clamp(corner_radii, vec2(0.0), vec2(min(caster.x, caster.y) * 0.5));
    float dist = rounded_rect_sdf(p, caster, radii);
    float hole_dist = rounded_rect_sdf(v_coords * size - hole_center, caster, radii);

    float blur = max(shadow_radius, 1.0);
    float outset = max(spread, 0.0);
    if (hole_dist < -0.75 || max(dist, 0.0) >= outset + blur * 3.0) {
        discard;
    }

    float sigma = max(blur * 0.5, 0.5);
    float falloff = 0.5 * (
        1.0 - erf_approx((dist - outset) / (sigma * 1.41421356))
    );
    float cut = smoothstep(-0.75, 0.75, hole_dist);
    float a = shadow_color.a * alpha * falloff * cut;
    if (a <= 0.003) {
        discard;
    }
    gl_FragColor = vec4(shadow_color.rgb * a, a);
}
