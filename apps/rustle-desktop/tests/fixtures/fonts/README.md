# SDF test font

RustleSdfTest.ttf is a renamed, regular-weight (400) subset of Noto Sans SC.
It contains space, A, 你 and あ for the SDF rasterization tests. It is loaded
only by tests; production font selection is unchanged.

- Source: https://github.com/google/fonts/tree/a85815a42757630ce188fdad368c2dfc444d4773/ofl/notosanssc
- Original file: NotoSansSC[wght].ttf
- Original SHA-256: a3041811a78c361b1de50f953c805e0244951c21c5bd412f7232ef0d899af0da
- License: SIL Open Font License 1.1, included in OFL.txt.
- Generated using fontTools 4.60.1: subset to U+0020, U+0041, U+4F60,
  U+3042, instantiate wght=400, and rename family/full/PostScript names
  to Rustle SDF Test / RustleSdfTest-Regular.

Tests use an isolated font database so they require neither installed fonts
nor network access. Keep this fixture and its license together.
