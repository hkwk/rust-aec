# rust-aec 文档（中文）

## 一、简介

### 1.1 编写目的

- 说明 `rust-aec` crate 的使用方式、接口形态与注意事项。
- 帮助在 GRIB2 模板 5.0=42（CCSDS/AEC）等场景中，以纯 Rust 方式解码 AEC 负载数据。
- 作为移交/维护时的快速理解材料（包括 streaming/增量解码的推荐调用方式）。

### 1.2 使用对象

- Rust 开发者：需要在数据管线中解码 CCSDS 121.0-B-3 AEC bitstream。
- 气象/GRIB2 相关开发者：需要解码 GRIB2 Section 7 的 template 42 AEC payload。
- 需要避免 native 依赖（C/CMake/bindgen/libclang 等）的人群，尤其是 Windows 环境。

## 二、用途

### 2.1 功能介绍

`rust-aec` 是 **CCSDS 121.0-B-3 Adaptive Entropy Coding (AEC)** 的纯 Rust 解码实现，初始目标聚焦：

- **GRIB2 Data Representation Template 5.0 = 42 (CCSDS/AEC)**

它负责把 AEC 压缩 payload 解码成“按 sample 打包的字节序列”（packed sample bytes），并提供：

- one-shot 解码：一次性输入、一次性输出。
- streaming 解码：分块喂输入、分块取输出，语义参考 `libaec` 的 `aec_stream`。

### 2.2 功能清单

- AEC 解码：
  - `decode()` / `decode_into()`：one-shot 解码 API。
  - `Decoder`：streaming/增量输出解码器（`push_input` + `decode` 循环）。
- 参数与标志：
  - `AecParams`：`bits_per_sample` / `block_size` / `rsi` / `flags`。
  - `AecFlags`：包含 `DATA_PREPROCESS`、`MSB`、`DATA_SIGNED` 等。
  - `flags_from_grib2_ccsds_flags()`：从 GRIB2 template 5.42 的 `ccsdsFlags` 映射到 `AecFlags`。
- 测试验证：
  - Oracle 测试（可选）：当仓库根目录存在 `aec_payload.bin` 与 `aec_decoded_oracle.bin` 时，做 byte-for-byte 对比。
  - Streaming vs one-shot 等价性测试（同样在 payload 文件存在时运行）。

## 三、运行环境说明

### 3.1 软件环境

- Rust：MSRV 为 **1.85**（见 `Cargo.toml` 的 `rust-version`）。
- 构建工具：标准 Rust toolchain（`cargo test` / `cargo run`）。

### 3.2 外部依赖

- 运行时：无外部系统依赖（纯 Rust）。
- crates 依赖：
  - `bitflags`（核心依赖）
  - `anyhow`（仅 dev-dependencies：示例/测试使用）

### 3.3 配置文件

- 本 crate **不需要**配置文件。
- 可选数据文件（用于本地验证，不要求提交/不要求在 CI 存在）：
  - 仓库根目录 `aec_payload.bin`：待解码的 AEC payload（例如来自 GRIB2 Section 7）。
  - 仓库根目录 `aec_decoded_oracle.bin`：用 `libaec` 或其他权威实现生成的 oracle 输出。

> 说明：`tests/oracle_data_grib2.rs` 与 `tests/streaming_decoder.rs` 在缺少上述文件时会自动跳过。

## 四、接口说明

### 4.1 One-shot API

- `decode(input: &[u8], params: AecParams, output_samples: usize) -> Result<Vec<u8>, AecError>`
  - 输入：AEC bitstream（payload）。
  - 输出：长度为 `output_samples * bytes_per_sample` 的 `Vec<u8>`。

- `decode_into(input: &[u8], params: AecParams, output_samples: usize, output: &mut [u8]) -> Result<(), AecError>`
  - 与 `decode` 语义相同，但由调用方提供输出 buffer（便于复用内存）。
  - `output.len()` 必须等于 `output_samples * bytes_per_sample`。

`bytes_per_sample` 通常为：

- `ceil(bits_per_sample / 8)`

并受 `AecFlags::DATA_3BYTE` 等规则影响。

### 4.2 Streaming API（增量输出）

> 目标：在输入/输出都可能被分块的场景（网络流、分片读取、边解码边消费）中使用。

- 构造：`Decoder::new(params, output_samples) -> Result<Decoder, AecError>`
- 输入：`Decoder::push_input(&mut self, input: &[u8])`
- 解码：
  - `Decoder::decode(&mut self, out: &mut [u8], flush: Flush) -> Result<(usize, DecodeStatus), AecError>`
  - 返回 `(written_bytes, status)`。

状态枚举：

- `DecodeStatus::NeedInput`：需要更多输入字节才可继续。
- `DecodeStatus::NeedOutput`：输出缓冲区满了，需要更大的/下一块输出 buffer。
- `DecodeStatus::Finished`：已经产出 `output_samples` 所需的全部输出。

Flush 枚举：

- `Flush::NoFlush`：允许后续继续 `push_input`。
- `Flush::Flush`：调用方声明不会再提供更多输入；如果此时数据不足以完成解码，会返回错误。

统计信息：

- `total_in()`：累计消耗的输入字节数。
- `total_out()`：累计产出的输出字节数。
- `avail_in()`：当前内部缓冲区中可供读取的输入字节数。

### 4.3 GRIB2 参数映射

- `flags_from_grib2_ccsds_flags(ccsds_flags: u8) -> AecFlags`
  - 将 GRIB2 template 5.42 的 `ccsdsFlags` 映射到 `AecFlags`。

使用时你仍需要从 GRIB2 模板中读到：

- `bits_per_sample`
- `block_size`
- `rsi`
- `ccsds_flags`
- `output_samples`（GRIB2 通常来自 Section 5 的 `num_encoded_points`）

## 五、架构说明

### 5.1 总体架构

模块划分（以 `src/` 为准）：

- `lib.rs`：对外 API（re-export、one-shot 函数、flags 映射）。
- `decoder.rs`：核心解码逻辑（one-shot + streaming 解码器）。
- `bitreader.rs`：MSB-first 位读取器。
- `params.rs`：参数与 flag 定义。
- `error.rs`：错误类型。

### 5.5 关键设计点

- **纯 Rust**：不依赖 `libaec`，避免 native 构建链路问题。
- **输出语义清晰**：输出为“按 sample 打包的字节流”，便于上层按 `bits_per_sample` 重新解释为整数样本。
- **与 libaec 对齐的行为**：
  - 支持预处理（`DATA_PREPROCESS`）的逆变换，输出为重构后的样本值。
  - Streaming API 提供 `NeedInput/NeedOutput/Finished` 与 `Flush` 语义，便于用与 `libaec` 类似的驱动循环集成。
- **可选 oracle 验证**：仓库不强制携带大体积二进制测试数据；本地放置文件即可验证 byte-for-byte。

## 六、使用说明

### 6.1 作为库使用（one-shot）

```rust
use rust_aec::{decode, flags_from_grib2_ccsds_flags, AecParams};

let params = AecParams::new(
    12,                 // bits_per_sample
    32,                 // block_size
    128,                // rsi
    flags_from_grib2_ccsds_flags(0x0e),
);

let decoded: Vec<u8> = decode(&payload, params, num_points)?;
```

### 6.2 作为库使用（streaming/增量）

驱动循环示意：

- 按块 `push_input()`
- 反复调用 `decode()` 直到 `NeedInput` 或 `Finished`
- 输入结束后用 `Flush::Flush` 进入收尾阶段

仓库内可直接运行示例：

```powershell
cargo run -p rust-aec --example stream_decode_aec_payload -- --payload aec_payload.bin --samples 1038240
```

可调入参：

- `--in-chunk <n>`：每次喂入的输入字节数
- `--out-chunk <n>`：每次提供的输出缓冲区大小

### 6.3 运行示例（one-shot）

```powershell
cargo run -p rust-aec --example decode_aec_payload -- --payload aec_payload.bin --samples 1038240
```

### 6.4 运行测试

```powershell
cargo test
```

- 若仓库根目录存在 `aec_payload.bin` 与 `aec_decoded_oracle.bin`：会运行 oracle 对比测试。
- 若只存在 `aec_payload.bin`：会运行 streaming vs one-shot 等价性测试。
- 若都不存在：相关测试会跳过，不影响 CI。

## 截图占位

- （截图占位）Streaming 示例运行输出
- （截图占位）Oracle 测试通过输出
- （截图占位）与 `libaec` 对比图（可参考仓库 README 的可视化对比）
