//! 界面基础组件；GPUI 缺失的通用控件在此补齐。

pub(crate) mod text_input;

pub(crate) use text_input::{TextInput, bind_input_keys};
