# Signatures only: the arguments are used by the compiled extension.
# pylint: disable=unused-argument
def version_banner() -> str: ...
def pr_view(
    owner: str,
    repo: str,
    number: int,
    fields: list[str],
    hostname: str | None = None,
) -> str: ...
def gh_read(args: list[str]) -> tuple[bytes, bytes, int, str, bool]: ...
