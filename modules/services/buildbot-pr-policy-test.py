from buildbot.changes.changes import Change

from buildbot.plugins import schedulers, util
from buildbot.plugins import worker as worker_plugin
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
    # A real Change, which is exactly what BaseScheduler._changeCallback
    # passes to the filter. A SimpleNamespace(author=...) stub hid a bug:
    # the attribute is `who`, so the filter raised AttributeError in
    # production while every stubbed assertion here passed.
    return Change(
        who=author,
        files=[],
        comments="",
        revision="0" * 40,
        when=1787848101,
        branch=branch,
        category=category,
        revlink="",
        properties={},
        repository=repository,
        codebase="",
        project="bipa-app/bipa",
    )


# regression guard: the filter must read the attribute a real Change exposes
assert not hasattr(change("nicolasauler", "pull", "refs/pull/1/merge"), "author")
assert change("nicolasauler", "pull", "refs/pull/1/merge").who == "nicolasauler"


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


def builder(name):
    return util.BuilderConfig(
        name=name, workernames=["desktop-000"], factory=util.BuildFactory()
    )


def build_locks(**policy_args):
    config = {
        "schedulers": [basic("bipa-app-bipa-prs", category="pull")],
        "workers": [worker_plugin.Worker("desktop-000", "pass")],
        "builders": [
            builder("bipa-app/bipa/nix-eval"),
            builder("bipa-app/bipa/nix-build"),
            builder("bipa-app/bipa/nix-register-gcroot"),
            builder("nicolasauler/ninja/nix-build"),
        ],
    }
    WorkstationPolicyConfigurator(
        pr_authors=["nicolasauler"], build_pushes=False, **policy_args
    ).configure(config)
    assert [configured.max_builds for configured in config["workers"]] == [None], (
        "worker build slots must stay uncapped; see the deadlock guard below"
    )
    return {configured.name: configured.locks for configured in config["builders"]}


# upstream shape: no locks anywhere
assert all(locks == [] for locks in build_locks().values())
assert all(locks == [] for locks in build_locks(max_concurrent_nix_builds=None).values())

locks = build_locks(max_concurrent_nix_builds=1)
# only the compiling builders are serialised; eval and gcroot must stay free,
# or the eval build can never reach the step that triggers a nix build
assert locks["bipa-app/bipa/nix-eval"] == []
assert locks["bipa-app/bipa/nix-register-gcroot"] == []
# and every project shares ONE semaphore of one, across the whole master
[bipa_access] = locks["bipa-app/bipa/nix-build"]
[ninja_access] = locks["nicolasauler/ninja/nix-build"]
assert bipa_access.mode == "counting"
assert bipa_access.lockid.maxCount == 1
# identity, not equality: buildbot rejects a config where two locks share a
# name but are different objects (config/master.py:948-950)
assert bipa_access.lockid is ninja_access.lockid
assert build_locks(max_concurrent_nix_builds=2)[
    "bipa-app/bipa/nix-build"
][0].lockid.maxCount == 2

for invalid in (0, -1):
    try:
        WorkstationPolicyConfigurator(
            pr_authors=["nicolasauler"],
            build_pushes=False,
            max_concurrent_nix_builds=invalid,
        )
    except ValueError:
        pass
    else:
        raise AssertionError("non-positive max_concurrent_nix_builds must fail closed")

try:
    WorkstationPolicyConfigurator(
        pr_authors=["nicolasauler"], build_pushes=False, max_concurrent_nix_builds=1
    ).configure(
        {
            "schedulers": [basic("bipa-app-bipa-prs", category="pull")],
            "builders": [builder("bipa-app/bipa/nix-eval")],
        }
    )
except RuntimeError:
    pass
else:
    raise AssertionError("a renamed nix-build builder must fail closed")


def bootstrap_locks(builders):
    # A master with no registered projects: buildbot-nix still adds its reload
    # builder unconditionally (buildbot_nix/__init__.py:188,
    # github_projects.py:659), but no project builders exist yet. This is the
    # state after a fresh deploy, a wiped github-project-cache, or the App
    # losing access -- the configurator must not abort master.cfg, or the
    # master can never come back to discover projects.
    config = {
        "schedulers": [],
        "workers": [worker_plugin.Worker("desktop-000", "pass")],
        "builders": builders,
    }
    WorkstationPolicyConfigurator(
        pr_authors=["nicolasauler"], build_pushes=False, max_concurrent_nix_builds=1
    ).configure(config)
    return [configured.locks for configured in config["builders"]]


assert bootstrap_locks([builder("reload-github-projects")]) == [[]]
assert bootstrap_locks([]) == []


# Deadlock guard. This is why the cap is a builder lock and not Worker.max_builds.
#
# buildbot-nix puts every builder of a project on the same worker list
# (buildbot_nix/project_config.py:160-199), and a nix-eval build parks in its
# BuildTrigger step until the nix-build builds it triggered have finished
# (buildbot_nix/build_trigger.py:771), holding its slot the whole time. So the
# worker must be able to run the eval build and the build it triggered at once.
# Worker.canStartBuild counts busy builds across ALL builders
# (buildbot worker/base.py:657-659) -- with max_builds=1 it never can.
class BusyForBuilder:
    def __init__(self, busy):
        self.busy = busy

    def isBusy(self):
        return self.busy


def admits_triggered_build(max_builds):
    configured = worker_plugin.Worker("desktop-000", "pass")
    configured.max_builds = max_builds
    configured.locks = []
    configured.workerforbuilders = {
        # parked in BuildTrigger, waiting for the build below
        "bipa-app/bipa/nix-eval": BusyForBuilder(True),
        "bipa-app/bipa/nix-build": BusyForBuilder(False),
    }
    return configured.canStartBuild()


assert not admits_triggered_build(1), "max_builds=1 deadlocks buildbot-nix fan-out"
# raising the cap only moves the wall: max_builds=2 admits this build, but a
# second open PR parks a second eval build and the wall is back. Nothing bounds
# how many eval builds park, so no finite per-worker cap is safe.
assert admits_triggered_build(2)
assert admits_triggered_build(None), "uncapped workers must admit the triggered build"
