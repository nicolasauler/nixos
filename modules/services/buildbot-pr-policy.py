from __future__ import annotations

from collections.abc import Iterable
from typing import Any

from buildbot.changes.filter import ChangeFilter
from buildbot.configurators import ConfiguratorBase
from buildbot.schedulers.basic import BaseBasicScheduler


class PullRequestAuthorFilter(ChangeFilter):
    compare_attrs = ("inner_filter", "pr_authors")

    def __init__(self, inner_filter: ChangeFilter, pr_authors: frozenset[str]) -> None:
        super().__init__()
        self.inner_filter = inner_filter
        self.pr_authors = pr_authors

    def filter_change(self, change: Any) -> bool:
        return (
            self.inner_filter.filter_change(change)
            and change.category == "pull"
            and (change.author or "").strip().lower() in self.pr_authors
        )


class WorkstationPolicyConfigurator(ConfiguratorBase):
    def __init__(
        self,
        *,
        pr_authors: Iterable[str],
        build_pushes: bool,
        max_builds_per_worker: int | None = None,
    ) -> None:
        super().__init__()
        self.pr_authors = frozenset(
            author.strip().lower() for author in pr_authors if author.strip()
        )
        if not self.pr_authors:
            raise ValueError("pr_authors must not be empty")
        self.build_pushes = build_pushes
        if max_builds_per_worker is not None and max_builds_per_worker < 1:
            raise ValueError("max_builds_per_worker must be >= 1 when set")
        self.max_builds_per_worker = max_builds_per_worker

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

        # buildbot-nix constructs workers as `worker.Worker(name, password)`
        # (buildbot_nix/__init__.py:116) and never passes max_builds, whose
        # default means UNLIMITED concurrent builds per worker. So a workersFile
        # `cores: N` caps worker *processes*, not builds: this box ran 8 builds
        # on 4 workers and starved. buildbot reads max_builds at decision time
        # (worker/base.py canStartBuild), so setting it here is honoured, and
        # configurators re-run on reconfig. Applied to every worker without
        # inspecting names or types, so an upstream rename cannot silently
        # un-cap us (contrast the -prs guard above).
        if self.max_builds_per_worker is not None:
            for configured_worker in self.workers:
                configured_worker.max_builds = self.max_builds_per_worker
