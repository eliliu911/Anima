//! 拿**真实的深目录**验一遍按需取子树。
//!
//! 单测用的是临时目录，形状是我自己造的；这条打的是用户实际看到出问题的
//! 那个路径（`novels/ai-last-command/vol1-余波/剧本/`，内容在第 5 层）。
//! 默认 `#[ignore]` —— 它依赖本机上存在那个工作区。

const WS: &str = "/Users/you/AI/workspace/MyAIMovie";
const DEEP: &str = "novels/ai-last-command/vol1-余波/剧本";

fn find<'a>(
    nodes: &'a [anima_lib::workspace::FileTreeNode],
    path: &str,
) -> Option<&'a anima_lib::workspace::FileTreeNode> {
    for n in nodes {
        if n.path == path {
            return Some(n);
        }
        if let Some(kids) = &n.children {
            if let Some(hit) = find(kids, path) {
                return Some(hit);
            }
        }
    }
    None
}

#[tokio::test]
#[ignore = "需要本机存在 MyAIMovie 工作区"]
async fn 第五层的目录自报还有内容而且能按需取到() {
    if !std::path::Path::new(WS).join(DEEP).is_dir() {
        eprintln!("跳过：{WS}/{DEEP} 不存在");
        return;
    }

    // 第一次：默认深度，剧本 在第 4 层，它的内容取不到
    let top = anima_lib::workspace::workspace_list_directory_tree(Some(WS.into()), None, None)
        .await
        .unwrap();
    let deep = find(&top, DEEP).expect("剧本 这一层应当在树里");
    assert!(deep.children.is_none(), "第 5 层不该一次带回来");
    assert!(
        deep.truncated,
        "剧本 里有 EP01~EP06，必须自报还有内容 —— 否则界面上和空目录一样"
    );

    // 第二次：前端展开它时按路径再取一层
    let sub =
        anima_lib::workspace::workspace_list_directory_tree(Some(WS.into()), Some(DEEP.into()), None)
            .await
            .unwrap();
    assert!(!sub.is_empty(), "按需取应当拿到 EP01~EP06");
    // 路径仍然相对工作区根 —— 前端拿它当 key、也拿它做下一次的 sub_path
    for n in &sub {
        assert!(
            n.path.starts_with(&format!("{DEEP}/")),
            "路径应当相对工作区根，实际：{}",
            n.path
        );
    }
    println!(
        "按需取回 {} 项：{:?}",
        sub.len(),
        sub.iter().map(|n| &n.name).collect::<Vec<_>>()
    );
}
