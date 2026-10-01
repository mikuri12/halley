precision highp float;
//_DEFINES

varying vec2 v_coords;
uniform sampler2D tex;
uniform float alpha;
uniform vec2 halfpixel;
uniform float offset;

void main() {
    vec2 uv = clamp(v_coords, vec2(0.0), vec2(1.0));
    vec2 o = halfpixel * offset;

    vec4 sum = texture2D(tex, uv) * 4.0;
    sum += texture2D(tex, uv - o);
    sum += texture2D(tex, uv + o);
    sum += texture2D(tex, uv + vec2(o.x, -o.y));
    sum += texture2D(tex, uv - vec2(o.x, -o.y));

    gl_FragColor = (sum / 8.0) * alpha;
}
