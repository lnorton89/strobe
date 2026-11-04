// `undeclaredHere` is declared by nothing, so this must not compile.
vec4 renderMain(void) {
    return vec4(undeclaredHere, 0.0, 0.0, 1.0);
}
