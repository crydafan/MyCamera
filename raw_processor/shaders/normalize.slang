RWTexture2D<uint16_t> RawShifted;
RWTexture2D<half> RawNormalized;

[push_constant]
cbuffer Uniforms {
  float4 colorGains;
  int4 blackLevel;
  uint whiteLevel;
}

[Shader("compute")]
[NumThreads(8, 8, 1)]
void computeMain(uint3 threadId: SV_DispatchThreadID) {
  int x = threadId.x;
  int y = threadId.y;

  // Assuming RGGB pattern: [R G; G B]
  uint xPhase = x & 1;
  uint yPhase = y & 1;
  uint index = yPhase * 2 + xPhase;

  // Remove sensor bias by subtracting the black level
  float norm = float(RawShifted[int2(x, y)] - blackLevel[index]) /
               float(whiteLevel - blackLevel[index]);

  RawNormalized[int2(x, y)] = half(norm * colorGains[index]);
}
