"""The pages, against a stand-in for the sidecar: each served at its level,
each drawing the custody role's sections and "not set up yet" states, each
tool answering the role's record."""

from __future__ import annotations

from dataclasses import dataclass, field

import meridian
from meridian.plugin.v1 import operations_pb2 as ops
from meridian.testing import PageClient

from reference_plugin.custodian import Custodian
from reference_plugin.page import pages, use


@dataclass
class Sidecar:
    """Who the plugin was launched as, and the accounts granted to it."""

    granted: list[ops.AccountRecord] = field(default_factory=list)
    identity = meridian.Identity("reference-1", roles=("custody",))

    async def read_accounts_for_linking(self, **_: object) -> ops.ReadAccountsForLinkingResult:
        return ops.ReadAccountsForLinkingResult(accounts=self.granted)

    async def link_external_account(self, **given: str) -> ops.LinkExternalAccountResult:
        return ops.LinkExternalAccountResult(
            external_account_id=given["external_account_id"], account_id=given["account_id"]
        )


def client(sidecar: Sidecar | None = None) -> PageClient:
    scope = meridian.AccountScope(
        read=frozenset({"ACC-1"}),
        links=(meridian.LinkedExternalAccount("REF-1001", "ACC-1", "Main"),),
    )
    use(Custodian(), key_set=False, scope=scope)
    return PageClient(pages, sidecar or Sidecar(), read={"ACC-1"})


def test_each_page_is_served_at_its_levels() -> None:
    served = {(r.page.path, r.level): r.response.status for r in client().every_page()}
    assert served[("/", "admin")] == 200 and served[("/", "write")] == 403
    assert served[("/links", "admin")] == 200
    assert served[("/statements", "write")] == 200 and served[("/statements", "admin")] == 403


def test_connections_say_the_next_step_until_the_key_is_set() -> None:
    page = client().get("/", "admin").text
    assert "No credentials yet" in page and "<h2>Connections</h2>" in page
    assert (
        client().call_tool("read_connections").data["connections"][0]["accounts_reached"] == 2
    )


def test_account_links_say_no_account_is_granted_until_one_is() -> None:
    assert "No accounts granted yet" in client().get("/links", "admin").text
    granted = Sidecar(granted=[ops.AccountRecord(account_id="ACC-1", name="Main")])
    page = client(granted).get("/links", "admin").text
    assert "No accounts granted yet" not in page and "Individual brokerage" in page


def test_statements_show_the_linked_accounts_the_person_may_read() -> None:
    page = client().get("/statements", "read").text
    assert "<strong>Main</strong>" in page and "Roth IRA" not in page


def test_an_agent_links_an_account_and_a_person_posts_the_same_form() -> None:
    granted = Sidecar(granted=[ops.AccountRecord(account_id="ACC-1", name="Main")])
    made = client(granted).call_tool(
        "link_account", {"external_account_id": "REF-1002", "account_id": "ACC-1"}
    )
    assert made.outcome == "made"
    assert made.data == {
        "external_account_id": "REF-1002",
        "account_id": "ACC-1",
        "account_name": "",
    }
    posted = client(granted).post(
        "/links", "admin", {"external_account_id": "REF-1002", "account_id": "ACC-1"}
    )
    assert posted.status == 200 and "<om-account-map" in posted.text


def test_history_shows_the_activity_of_the_linked_accounts_the_person_may_read() -> None:
    page = client().get("/history", "read").text
    assert "<h2>History</h2>" in page and "<strong>Main</strong>" in page
    answered = client().call_tool("read_account_activities")
    assert [each["account"] for each in answered.data["activities"]] == ["Main"]
