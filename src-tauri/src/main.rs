// release 构建下不弹出多余的终端窗口（Windows），macOS 上无副作用
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    anima_lib::run()
}
