//! 情感状态按作用域分档：好感度全局，心情与活跃度每技能一套。
//!
//! 见 `docs/PERSONALITY_MEMORY_SKILLS.md` 3.2。这几条语义是整个设计的要点 ——
//! 写错了不会有任何报错，只会让"换角色时关系归零"或者"各角色情绪串味"，
//! 而那两种都很难从现象反推到原因。

use anima_lib::memory::store::Store;
use anima_lib::personality::EmotionState;

async fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("m.db")).expect("建库失败");
    (dir, store)
}

#[tokio::test]
async fn affection_is_shared_across_roles() {
    // **好感度是"我们之间的关系"**，不该因为换了身份而归零。
    // 这是分档设计里最重要的一条
    let (_dir, store) = store().await;
    let doc = Store::skill_scope(Some("family-doctor"));
    let eng = Store::skill_scope(Some("software-engineer"));

    store
        .set_emotion_for(
            &doc,
            EmotionState {
                affection: 72.0,
                mood: 30.0,
                energy: 60.0,
            },
        )
        .await
        .unwrap();

    // 换到另一个角色：好感度必须还在
    let after = store.emotion_for(&eng).await.unwrap();
    assert_eq!(after.affection, 72.0, "换角色不该让关系归零");
    // 全局那一份也看得到
    assert_eq!(store.emotion().await.unwrap().affection, 72.0);
}

#[tokio::test]
async fn mood_and_energy_are_per_role() {
    // "刚处理完一个急诊"和"刚聊完游戏"的即时状态本来就不同
    let (_dir, store) = store().await;
    let doc = Store::skill_scope(Some("family-doctor"));
    let counselor = Store::skill_scope(Some("counselor"));

    store
        .set_emotion_for(
            &doc,
            EmotionState {
                affection: 60.0,
                mood: -40.0,
                energy: 85.0,
            },
        )
        .await
        .unwrap();
    store
        .set_emotion_for(
            &counselor,
            EmotionState {
                affection: 60.0,
                mood: 45.0,
                energy: 20.0,
            },
        )
        .await
        .unwrap();

    let d = store.emotion_for(&doc).await.unwrap();
    let c = store.emotion_for(&counselor).await.unwrap();
    assert_eq!(d.mood, -40.0);
    assert_eq!(d.energy, 85.0);
    assert_eq!(c.mood, 45.0);
    assert_eq!(c.energy, 20.0);
    assert_eq!(d.affection, c.affection, "好感度仍然是共享的");
}

#[tokio::test]
async fn a_new_role_inherits_the_current_mood_rather_than_resetting() {
    // 切个角色就情绪突然归零会显得很怪，所以第一次用某个技能时继承全局的即时状态
    let (_dir, store) = store().await;
    store
        .set_emotion(EmotionState {
            affection: 55.0,
            mood: -25.0,
            energy: 70.0,
        })
        .await
        .unwrap();

    let fresh = store
        .emotion_for(&Store::skill_scope(Some("never-used")))
        .await
        .unwrap();
    assert_eq!(fresh.mood, -25.0, "第一次用应当继承当前心情，而不是回到 0");
    assert_eq!(fresh.energy, 70.0);
    assert_eq!(fresh.affection, 55.0);
}

#[tokio::test]
async fn writing_from_a_role_does_not_clobber_the_default_mood() {
    // global 行的心情与活跃度是"没扮演角色时"的即时情绪。
    // 在某个技能里聊一轮不该把它改写掉 —— 这一条靠 UPSERT 只更新 affection 列保证
    let (_dir, store) = store().await;
    store
        .set_emotion(EmotionState {
            affection: 50.0,
            mood: 10.0,
            energy: 55.0,
        })
        .await
        .unwrap();

    store
        .set_emotion_for(
            &Store::skill_scope(Some("family-doctor")),
            EmotionState {
                affection: 58.0,
                mood: -80.0,
                energy: 95.0,
            },
        )
        .await
        .unwrap();

    let global = store.emotion().await.unwrap();
    assert_eq!(global.affection, 58.0, "好感度应当被更新");
    assert_eq!(global.mood, 10.0, "默认状态的心情不该被角色里的一轮改写");
    assert_eq!(global.energy, 55.0);
}

#[tokio::test]
async fn no_skill_means_the_global_scope() {
    assert_eq!(Store::skill_scope(None), Store::GLOBAL_SCOPE);
    // 空串和纯空白也算"没角色" —— 配置里留空是常见情况
    assert_eq!(Store::skill_scope(Some("")), Store::GLOBAL_SCOPE);
    assert_eq!(Store::skill_scope(Some("   ")), Store::GLOBAL_SCOPE);
    assert_eq!(Store::skill_scope(Some("counselor")), "skill:counselor");
}

#[tokio::test]
async fn each_scope_ages_on_its_own_clock() {
    // 一个一周没用的角色，它的心情该衰减一周，而不是跟着别的角色的活跃度走
    let (_dir, store) = store().await;
    let a = Store::skill_scope(Some("a"));
    store
        .set_emotion_for(
            &a,
            EmotionState {
                affection: 50.0,
                mood: 40.0,
                energy: 60.0,
            },
        )
        .await
        .unwrap();

    let (_, hours_a) = store.emotion_with_age_for(&a).await.unwrap();
    assert!(hours_a < 1.0, "刚写完，空闲时长应当接近 0：{hours_a}");

    // 没写过的作用域退回全局的时间戳（它继承的正是全局状态）
    let (_, hours_new) = store
        .emotion_with_age_for(&Store::skill_scope(Some("never")))
        .await
        .unwrap();
    assert!(hours_new < 1.0, "退回全局时间戳，也该接近 0：{hours_new}");
}
