// https://github.com/timothybrooks/hdr-plus/blob/373869c2708ebf7ed2ec85bc0af0ec38500bd56d/src/finish.cpp#L747
RWTexture2D<uint16_t> Raw;
RWTexture2D<uint16_t> RawShifted;

[push_constant]
cbuffer Uniform { int2 shiftVector; }

[Shader("compute")]
[NumThreads(8, 8, 1)]
void computeMain(uint2 threadId: SV_DispatchThreadID) {
  int2 coordinates = threadId.xy;
  RawShifted[coordinates] = Raw[coordinates + shiftVector];
}
