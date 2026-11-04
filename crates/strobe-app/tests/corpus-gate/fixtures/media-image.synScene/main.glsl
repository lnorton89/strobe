// The image the scene declares under IMAGES, tiled twice and tinted, so a
// decode, upload or sampler-binding regression changes the frame.
vec4 renderMain(void) {
    vec4 mark = texture(mark, fract(_uv * 2.0));
    return vec4(mark.rgb * vec3(1.0, 0.8, 0.6) + 0.1 * _uv.x, 1.0);
}
