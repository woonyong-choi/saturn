use super::*;

const ROOT: ChatId = ChatId(1);

fn token(text: &str) -> PassToken {
    PassToken::new(text.to_owned())
}

fn limits(max_depth: u32, max_concurrent: u32, max_total: u32) -> PassLimits {
    PassLimits {
        max_depth,
        max_concurrent,
        max_total,
    }
}

fn table(limits: PassLimits) -> PassTable {
    let mut table = PassTable::new(limits);
    table.open_root(ROOT, Mode::Edit, token("root"));
    table
}

fn admitted(admission: Admission) -> Grant {
    match admission {
        Admission::Admitted(grant) => grant,
        other => panic!("expected admitted, got {other:?}"),
    }
}

/// 요청을 허용받고 하위 채팅과 출입증을 묶는다.
fn open_child(table: &mut PassTable, parent: &str, waiter: u64, chat: u64) -> Grant {
    let grant = admitted(table.request(Waiter(waiter), &token(parent), None));
    table.bind(&grant, ChatId(chat), token(&format!("child-{chat}")));
    grant
}

#[test]
fn unknown_pass_is_rejected() {
    let mut table = table(PassLimits::DEFAULT);

    let admission = table.request(Waiter(1), &token("guess"), None);

    assert_eq!(admission, Admission::Rejected(Reject::UnknownPass));
}

#[test]
fn child_is_one_level_below_its_parent_and_inherits_the_parent_mode() {
    let mut table = table(PassLimits::DEFAULT);

    let grant = admitted(table.request(Waiter(1), &token("root"), None));

    assert_eq!(grant.parent, ROOT);
    assert_eq!(grant.depth, 1);
    assert_eq!(grant.mode, Mode::Edit);
}

#[test]
fn request_above_the_parent_mode_is_rejected() {
    let mut table = table(PassLimits::DEFAULT);

    let admission = table.request(Waiter(1), &token("root"), Some(Mode::Full));

    assert_eq!(
        admission,
        Admission::Rejected(Reject::ModeAboveParent { parent: Mode::Edit })
    );
    assert_eq!(table.running_total(), 0);
}

#[test]
fn request_below_the_parent_mode_is_honored() {
    let mut table = table(PassLimits::DEFAULT);

    let grant = admitted(table.request(Waiter(1), &token("root"), Some(Mode::ReadOnly)));

    assert_eq!(grant.mode, Mode::ReadOnly);
}

#[test]
fn grandchild_is_limited_by_the_parent_mode_not_the_root_mode() {
    let mut table = table(PassLimits::DEFAULT);
    let grant = admitted(table.request(Waiter(1), &token("root"), Some(Mode::Ask)));
    table.bind(&grant, ChatId(2), token("child-2"));

    let admission = table.request(Waiter(2), &token("child-2"), Some(Mode::Edit));

    assert_eq!(
        admission,
        Admission::Rejected(Reject::ModeAboveParent { parent: Mode::Ask })
    );
}

#[test]
fn depth_above_the_limit_is_rejected() {
    let mut table = table(limits(2, 8, 32));
    open_child(&mut table, "root", 1, 2);
    open_child(&mut table, "child-2", 2, 3);

    let admission = table.request(Waiter(3), &token("child-3"), None);

    assert_eq!(
        admission,
        Admission::Rejected(Reject::DepthExceeded { max: 2 })
    );
    assert_eq!(table.depth_of(ChatId(3)), Some(2));
}

#[test]
fn depth_limit_of_zero_refuses_every_child() {
    let mut table = table(limits(0, 8, 32));

    let admission = table.request(Waiter(1), &token("root"), None);

    assert_eq!(
        admission,
        Admission::Rejected(Reject::DepthExceeded { max: 0 })
    );
}

#[test]
fn request_over_the_concurrent_limit_waits_in_order() {
    let mut table = table(limits(2, 2, 32));
    open_child(&mut table, "root", 1, 2);
    open_child(&mut table, "root", 2, 3);

    let third = table.request(Waiter(3), &token("root"), None);
    let fourth = table.request(Waiter(4), &token("root"), None);

    assert_eq!(third, Admission::Queued { position: 1 });
    assert_eq!(fourth, Admission::Queued { position: 2 });
    assert_eq!(table.running_of(ROOT), 2);
}

#[test]
fn finished_child_hands_its_place_to_the_first_waiter() {
    let mut table = table(limits(2, 1, 32));
    open_child(&mut table, "root", 1, 2);
    table.request(Waiter(2), &token("root"), None);
    table.request(Waiter(3), &token("root"), None);

    let ended = table.end(ChatId(2));

    assert_eq!(ended.promoted.len(), 1);
    assert_eq!(ended.promoted[0].waiter, Waiter(2));
    assert_eq!(table.queued_total(), 1);
    assert_eq!(table.running_of(ROOT), 1);
}

#[test]
fn total_limit_holds_requests_of_other_parents_too() {
    let mut table = table(limits(3, 8, 2));
    open_child(&mut table, "root", 1, 2);
    open_child(&mut table, "child-2", 2, 3);

    let admission = table.request(Waiter(3), &token("root"), None);

    assert_eq!(admission, Admission::Queued { position: 1 });
    assert_eq!(table.running_total(), 2);
}

#[test]
fn waiter_blocked_by_its_own_parent_does_not_hold_back_another_parent() {
    let mut table = table(limits(3, 1, 8));
    open_child(&mut table, "root", 1, 2);
    table.request(Waiter(2), &token("root"), None);

    let other = table.request(Waiter(3), &token("child-2"), None);

    assert!(matches!(other, Admission::Admitted(_)));
}

#[test]
fn abandoned_grant_gives_the_place_back() {
    let mut table = table(limits(2, 1, 32));
    let grant = admitted(table.request(Waiter(1), &token("root"), None));
    table.request(Waiter(2), &token("root"), None);

    let ended = table.abandon(&grant);

    assert_eq!(ended.promoted.len(), 1);
    assert_eq!(ended.promoted[0].waiter, Waiter(2));
}

#[test]
fn cancelled_waiter_leaves_the_queue() {
    let mut table = table(limits(2, 1, 32));
    open_child(&mut table, "root", 1, 2);
    table.request(Waiter(2), &token("root"), None);

    assert!(table.cancel(Waiter(2)));

    assert_eq!(table.queued_total(), 0);
    assert!(!table.cancel(Waiter(2)));
}

#[test]
fn ending_children_returns_every_descendant_deepest_first_and_keeps_the_root_pass() {
    let mut table = table(limits(3, 8, 32));
    open_child(&mut table, "root", 1, 2);
    open_child(&mut table, "child-2", 2, 3);
    open_child(&mut table, "root", 3, 4);

    let ended = table.end_children(ROOT);

    assert_eq!(ended.chats.len(), 3);
    let position = |chat: u64| {
        ended
            .chats
            .iter()
            .position(|c| *c == ChatId(chat))
            .unwrap_or_else(|| panic!("chat {chat} should be ended"))
    };
    assert!(position(3) < position(2));
    assert_eq!(table.running_total(), 0);
    assert_eq!(table.lookup(&token("root")), Some(ROOT));
    assert_eq!(table.lookup(&token("child-3")), None);
}

#[test]
fn revoked_pass_is_unknown() {
    let mut table = table(PassLimits::DEFAULT);
    open_child(&mut table, "root", 1, 2);

    table.end_children(ROOT);

    assert_eq!(
        table.request(Waiter(2), &token("child-2"), None),
        Admission::Rejected(Reject::UnknownPass)
    );
}

#[test]
fn ending_children_cancels_waiters_under_the_parent() {
    let mut table = table(limits(2, 1, 32));
    open_child(&mut table, "root", 1, 2);
    table.request(Waiter(2), &token("root"), None);

    let ended = table.end_children(ROOT);

    assert_eq!(ended.cancelled, vec![Waiter(2)]);
    assert_eq!(table.queued_total(), 0);
}

#[test]
fn ending_a_middle_child_revokes_its_children_and_frees_its_place() {
    let mut table = table(limits(3, 8, 32));
    open_child(&mut table, "root", 1, 2);
    open_child(&mut table, "child-2", 2, 3);

    let ended = table.end(ChatId(2));

    assert_eq!(ended.chats, vec![ChatId(3)]);
    assert_eq!(table.running_total(), 0);
    assert_eq!(table.running_of(ROOT), 0);
    assert_eq!(table.lookup(&token("child-3")), None);
}

#[test]
fn raising_the_limits_promotes_waiters() {
    let mut table = table(limits(2, 1, 32));
    open_child(&mut table, "root", 1, 2);
    table.request(Waiter(2), &token("root"), None);

    let ended = table.set_limits(limits(2, 2, 32));

    assert_eq!(ended.promoted.len(), 1);
}

#[test]
fn check_mode_limits_a_child_to_its_parent_mode_and_leaves_roots_free() {
    let mut table = table(PassLimits::DEFAULT);
    let grant = admitted(table.request(Waiter(1), &token("root"), Some(Mode::Ask)));
    table.bind(&grant, ChatId(2), token("child-2"));

    assert_eq!(table.check_mode(ChatId(2), Mode::Full), Err(Mode::Edit));
    assert_eq!(table.check_mode(ChatId(2), Mode::Edit), Ok(()));
    assert_eq!(table.check_mode(ChatId(2), Mode::ReadOnly), Ok(()));
    assert_eq!(table.check_mode(ROOT, Mode::Full), Ok(()));
}

#[test]
fn lowering_the_parent_mode_lowers_the_ceiling_for_new_children() {
    let mut table = table(PassLimits::DEFAULT);
    table.set_mode(ROOT, Mode::ReadOnly);

    let grant = admitted(table.request(Waiter(1), &token("root"), None));

    assert_eq!(grant.mode, Mode::ReadOnly);
}

#[test]
fn token_text_never_appears_in_debug_output() {
    let mut table = PassTable::new(PassLimits::DEFAULT);
    table.open_root(ROOT, Mode::Edit, token("secret-pass-text"));

    assert!(!format!("{table:?}").contains("secret-pass-text"));
    assert!(!format!("{:?}", token("secret-pass-text")).contains("secret"));
}
