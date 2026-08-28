from types import SimpleNamespace

from buildbot.plugins import schedulers, util
from buildbot_pr_policy import WorkstationPolicyConfigurator

REPOSITORY = "https://github.com/bipa-app/bipa"
BUILDER = "bipa-app/bipa/nix-eval"


def basic(name: str, *, builder=BUILDER, **filter_args):
    return schedulers.SingleBranchScheduler(
        name=name,
        change_filter=util.ChangeFilter(repository=REPOSITORY, **filter_args),
        builderNames=[builder],
    )


def change(author, category, branch, repository=REPOSITORY):
    return SimpleNamespace(
        author=author,
        category=category,
        branch=branch,
        repository=repository,
        project="",
        codebase="",
        properties=SimpleNamespace(getProperty=lambda _name, default="": default),
    )


def configured_filter():
    config = {
        "schedulers": [
            basic("bipa-app-bipa-primary", filter_fn=lambda _change: True),
            basic("bipa-app-bipa-merge-queue", branch_re="gh-readonly-queue/.*"),
            basic("bipa-app-bipa-future-push", branch="release"),
            basic("bipa-app-bipa-prs", category="pull"),
            basic("unrelated-scheduler", builder="unrelated-builder", branch="main"),
            schedulers.ForceScheduler(
                name="bipa-app-bipa-force", builderNames=[BUILDER]
            ),
            schedulers.Triggerable(
                name="bipa-app-bipa-rebuild", builderNames=[BUILDER]
            ),
        ]
    }
    WorkstationPolicyConfigurator(
        pr_authors=[" NicolasAuler "], build_pushes=False
    ).configure(config)
    assert [scheduler.name for scheduler in config["schedulers"]] == [
        "bipa-app-bipa-prs",
        "unrelated-scheduler",
        "bipa-app-bipa-force",
        "bipa-app-bipa-rebuild",
    ]
    return config["schedulers"][0].getConfigDict()["kwargs"]["change_filter"]


pr_filter = configured_filter()
assert pr_filter.filter_change(change("nicolasauler", "pull", "refs/pull/1/merge"))
assert pr_filter.filter_change(change("NICOLASAULER", "pull", "refs/pull/1/merge"))
assert not pr_filter.filter_change(change("luizParreira", "pull", "refs/pull/2/merge"))
assert not pr_filter.filter_change(change(None, "pull", "refs/pull/3/merge"))
assert not pr_filter.filter_change(change("nicolasauler", None, "master"))
assert not pr_filter.filter_change(
    change(
        "nicolasauler",
        "pull",
        "refs/pull/1/merge",
        "https://github.com/other/repo",
    )
)
assert pr_filter == configured_filter()

push_config = {
    "schedulers": [
        basic("bipa-app-bipa-primary", filter_fn=lambda _change: True),
        basic("bipa-app-bipa-prs", category="pull"),
    ]
}
WorkstationPolicyConfigurator(
    pr_authors=["nicolasauler"], build_pushes=True
).configure(push_config)
assert [scheduler.name for scheduler in push_config["schedulers"]] == [
    "bipa-app-bipa-primary",
    "bipa-app-bipa-prs",
]
WorkstationPolicyConfigurator(
    pr_authors=["nicolasauler"], build_pushes=False
).configure({"schedulers": []})

try:
    WorkstationPolicyConfigurator(pr_authors=[], build_pushes=False)
except ValueError:
    pass
else:
    raise AssertionError("empty author policy must fail closed")

try:
    WorkstationPolicyConfigurator(
        pr_authors=["nicolasauler"], build_pushes=False
    ).configure({"schedulers": [basic("bipa-app-bipa-primary")]})
except RuntimeError:
    pass
else:
    raise AssertionError("changed upstream scheduler shape must fail closed")
