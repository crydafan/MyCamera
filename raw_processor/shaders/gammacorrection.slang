RWTexture2D<half4> Rgba;

[Shader("compute")]
[NumThreads(8, 8, 1)]
void computeMain(uint3 threadId: SV_DispatchThreadID) {
  uint2 coordinates = threadId.xy;

  float3 in = Rgba[coordinates].rgb;

  float3 l = 12.92 * in;
  float3 h =
      1.055 * pow(in, 0.41666666666 /* approximation of 1/2.4 */) - 0.055;
  float3 out = select(in <= 0.0031308, l, h);

  Rgba[coordinates] = half4(half3(out), 1.0h);
}
