RWTexture2D<half4> Rgba;
RWTexture2D<float4> Quantized;

[Shader("compute")]
[NumThreads(8, 8, 1)]
void computeMain(uint3 threadId: SV_DispatchThreadID) {
  uint2 coordinates = threadId.xy;
  // Quantization is done automatically by the device
  Quantized[coordinates] = Rgba[coordinates];
}
