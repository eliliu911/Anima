//! 数字人面容相关的服务端逻辑。
//!
//! 渲染在前端，这里只放那些**需要数据表、或需要测试守着**的部分：
//!
//! - [`viseme`]：文字 + 词边界 → 嘴型时间线（要拼音表，映射写错了不报错）
//! - [`model`]：模型文件的存放与读取（要挡路径穿越）

pub mod lipsync;
pub mod lipsync_stream;
pub mod model;
pub mod viseme;
