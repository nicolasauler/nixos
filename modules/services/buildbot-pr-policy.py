from __future__ import annotations

from collections.abc import Iterable
from typing import Any

from buildbot.changes.filter import ChangeFilter
from buildbot.configurators import ConfiguratorBase
from buildbot.locks import MasterLock
from buildbot.schedulers.basic import BaseBasicScheduler


class PullRequestAuthorFilter(ChangeFilter):
    compare_attrs = ("inner_filter", "pr_authors")

    def __init__(self, inner_filter: ChangeFilter, pr_authors: frozenset[str]) -> None:
        super().__init__()
        self.inner_filter = inner_filter
        self.pr_authors = pr_authors

    @staticmethod
    def _author_of(change: Any) -> str:
        # Schedulers are handed a Change object (schedulers/base.py:212,
        # `Change.fromChdict`), whose author field is named `who`
        # (changes.py:110) — there is no `.author`. Only the data-API dict
        # spells it `author`. Reading `.author` here raised AttributeError
        # *inside* the filter, so every pull-request change was silently
        # dropped: the change was recorded and no buildset was ever created.
        author = getattr(change, "who", None)
        if author is None:
            author = getattr(change, "author", None)
        return (author or "").strip().lower()

    def filter_change(self, change: Any) -> bool:
        return (
            self.inner_filter.filter_change(change)
            and change.category == "pull"
            and self._author_of(change) in self.pr_authors
        )


class WorkstationPolicyConfigurator(ConfiguratorBase):
    def __init__(
        self,
        *,
        pr_authors: Iterable[str],
        build_pushes: bool,
        max_concurrent_nix_builds: int | None = None,
    ) -> None:
        super().__init__()
        self.pr_authors = frozenset(
            author.strip().lower() for author in pr_authors if author.strip()
        )
        if not self.pr_authors:
            raise ValueError("pr_authors must not be empty")
        self.build_pushes = build_pushes
        if max_concurrent_nix_builds is not None and max_concurrent_nix_builds < 1:
            raise ValueError("max_concurrent_nix_builds must be >= 1 when set")
        self.max_concurrent_nix_builds = max_concurrent_nix_builds

    def _with_author_filter(self, scheduler: BaseBasicScheduler) -> BaseBasicScheduler:
        scheduler_config = scheduler.getConfigDict()
        scheduler_kwargs = dict(scheduler_config["kwargs"])
        inner_filter = scheduler_kwargs.get("change_filter")
        if not isinstance(inner_filter, ChangeFilter):
            raise RuntimeError(f"PR scheduler has no change filter: {scheduler.name}")
        scheduler_kwargs["change_filter"] = PullRequestAuthorFilter(
            inner_filter, self.pr_authors
        )
        return scheduler.__class__(
            *scheduler_config["args"],
            name=scheduler_config["name"],
            **scheduler_kwargs,
        )

    def configure(self, config: dict[str, Any]) -> None:
        super().configure(config)
        nix_eval_schedulers = [
            scheduler
            for scheduler in self.schedulers
            if isinstance(scheduler, BaseBasicScheduler)
            and any(builder.endswith("/nix-eval") for builder in scheduler.builderNames)
        ]
        pr_schedulers = [
            scheduler
            for scheduler in nix_eval_schedulers
            if scheduler.name.endswith("-prs")
        ]

        if nix_eval_schedulers and not pr_schedulers:
            raise RuntimeError("buildbot-nix created change schedulers but no -prs scheduler")

        replacements = {
            scheduler: self._with_author_filter(scheduler) for scheduler in pr_schedulers
        }
        self.config_dict["schedulers"] = [
            replacements.get(scheduler, scheduler)
            for scheduler in self.schedulers
            if self.build_pushes
            or scheduler not in nix_eval_schedulers
            or scheduler in replacements
        ]

        self._limit_nix_build_concurrency()

    def _limit_nix_build_concurrency(self) -> None:
        # Bound how many nix builds compile at once, WITHOUT bounding how many
        # builds a worker may run.
        #
        # Worker.max_builds is the wrong knob here and deadlocks buildbot-nix.
        # Every builder of a project shares one worker list
        # (buildbot_nix/project_config.py:160-199), and a nix-eval build parks
        # in its BuildTrigger step until the nix-build builds it triggered have
        # finished (buildbot_nix/build_trigger.py:771) -- holding its slot the
        # whole time (measured: 619s on desktop-001, build 1461 step "build
        # flake"). Worker.canStartBuild counts busy builds across ALL builders
        # (buildbot worker/base.py:657-659), so a worker capped at N builds
        # cannot admit the child its own N parked parents are waiting for.
        # Nothing bounds how many eval builds park at once -- one per open PR --
        # so no finite per-worker cap is safe.
        #
        # A master lock is the mechanism buildbot provides for this. Builder
        # locks are checked in Builder.canStartBuild (buildbot process/
        # builder.py:359-367) *before* a worker is marked busy, so a build that
        # cannot take the lock simply stays queued and consumes no slot. One
        # build per (builder, worker) pair is already guaranteed by buildbot
        # (process/builder.py:319-320), so this lock is what keeps the several
        # projects on this master from compiling at the same time.
        if self.max_concurrent_nix_builds is None:
            return

        nix_build_builders = [
            builder
            for builder in self.builders
            if getattr(builder, "name", "").endswith("/nix-build")
        ]
        if self.builders and not nix_build_builders:
            raise RuntimeError("buildbot-nix created builders but no */nix-build builder")

        access = MasterLock(
            "nix-build-concurrency", maxCount=self.max_concurrent_nix_builds
        ).access("counting")
        for builder in nix_build_builders:
            builder.locks = [*builder.locks, access]
