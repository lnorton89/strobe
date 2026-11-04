// Included by main.glsl: soft stripes across x.
float stripes(vec2 uv, float count) {
    return smoothstep(0.4, 0.6, fract(uv.x * count));
}
