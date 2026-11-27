use super::{TicketDraftParams, TicketInspectParams, TicketListParams, TicketUpdateParams};
use crate::schema::input_schema_for;
use crate::schema::tests::assert_keys;

#[test]
fn schemas_and_the_key_check_agree() {
    assert_keys(input_schema_for::<TicketListParams>(), &[]);
    assert_keys(input_schema_for::<TicketInspectParams>(), &["id"]);
    assert_keys(
        input_schema_for::<TicketDraftParams>(),
        &["attachments", "checks", "parent", "parts", "requirements", "text", "title", "view", "waits"],
    );
    assert_keys(input_schema_for::<TicketUpdateParams>(), &["action", "attachments", "id", "reason", "text", "to", "view"]);
}
