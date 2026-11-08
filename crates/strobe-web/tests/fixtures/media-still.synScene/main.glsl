// The scene's MEDIA picture, and whether one is showing (syn_MediaType
// is 0 without one), so a picture that never lands changes the frame.
vec4 renderMain(void) {
    vec3 picture = texture(syn_Media, _uv).rgb;
    return vec4(picture * 0.8 + 0.2 * syn_MediaType, 1.0);
}
