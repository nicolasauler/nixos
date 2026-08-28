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
    def __init__(self, *, pr_authors: Iterable[str], build_pushes: bool) -> None:
        super().__init__()
        self.pr_authors = frozenset(
            author.strip().lower() for author in pr_authors if author.strip()
        )
        if not self.pr_authors:
            raise ValueError("pr_authors must not be empty")
        self.build_pushes = build_pushes

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
