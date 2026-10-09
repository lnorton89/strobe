// Script Orbits — Strobe base library, scripted tier.
// The orbs' positions are not computed here: script.js simulates them and
// writes the result into the orb1..orb4 controls every frame. Without the
// script (the benchmark, which does not run scripts) they sit at their
// default compass points and the scene is still a valid picture.
//
// The spectrum is read through a macro because GLSL ES has no sampler1D:
// syn_Spectrum is a 1D texture on desktop GL and a one-texel-high 2D
// texture on the ANGLE path.

#ifdef GL_ES
#define SPEC(x) texture(syn_Spectrum, vec2(x, 0.5))
#else
#define SPEC(x) texture(syn_Spectrum, x)
#endif

// One orb: a hot core and a soft halo, tinted by its index around the hue.
vec3 orb(vec2 p, vec2 centre, float hue_offset) {
    float d = length(p - centre);
    float core = 0.0022 / (d * d + 0.0009);
    float halo = 0.22 * exp(-d * 6.0);
    vec3 tint = _hsv2rgb(vec3(fract(hue + hue_offset), 0.7, 1.0));
    return tint * (core + halo);
}

vec4 renderMain(void) {
    vec2 p = _uvc;
    // Control space is -1..1 across the picture; _uvc is centred and scaled
    // by height, so the wide axis is stretched by the aspect ratio.
    vec2 field = vec2(RENDERSIZE.x / RENDERSIZE.y, 1.0) * 0.5;

    vec3 col = orb(p, orb1 * field, 0.00)
             + orb(p, orb2 * field, 0.08)
             + orb(p, orb3 * field, 0.16)
             + orb(p, orb4 * field, 0.24);
    col *= glow * (0.7 + 0.8 * syn_Level);

    // The spectrum, as bars along the bottom edge.
    float level = SPEC(_uv.x).g;
    float top = 0.02 + level * 0.2;
    float bar = smoothstep(top, top - 0.012, _uv.y);
    col += _hsv2rgb(vec3(fract(hue + _uv.x * 0.3), 0.6, 1.0)) * bar * 0.5;

    // Trails: whatever was on screen last frame, faded.
    vec3 prev = texture(syn_FinalPass, _uv).rgb * trail;
    return vec4(max(col, prev), 1.0);
}
