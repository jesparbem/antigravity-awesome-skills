"""Execution back-ends. ``paper`` needs nothing; ``live`` delegates to the
private Polymarket order runner exactly as v1 did, but with guardrails."""

from .base import ExecutionEngine, OpenRequest
from .paper import PaperExecutor

__all__ = ["ExecutionEngine", "OpenRequest", "PaperExecutor"]
