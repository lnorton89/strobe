// Each frame keeps 90% of the last one and adds a dot that moves with
// time, so the result depends on the frames before it.
vec4 renderMain(void) {
    vec3 prev = texture(syn_FinalPass, _uv).rgb * 0.9;
    vec2 spot = vec2(0.3 + 0.1 * sin(TIME * 3.0), 0.5);
    float blob = smoothstep(0.08, 0.0, length(_uv - spot));
    return vec4(max(prev, vec3(blob, 0.5 * blob, 0.2)), 1.0);
}
