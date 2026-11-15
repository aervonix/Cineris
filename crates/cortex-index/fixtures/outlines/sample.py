"""A module docstring."""

import os

MAX_RETRIES = 3
DEFAULT_NAME: str = "x"
lowercase = 1


def top_level(a, b):
    """Adds two numbers."""
    def local_helper():
        pass
    return a + b


@dataclass
class Point:
    """A point on the plane.

    Longer text.
    """

    x: int = 0

    def __init__(self, x):
        self.x = x

    @property
    def norm(
        self,
        other=None,
    ) -> float:
        '''
        The length of the vector.
        '''
        return 0.0

    class Inner:
        def method(self):
            text = """
def not_a_function():
    pass
"""
            return text


async def fetch(url):  # fetches
    pass


# A comment about the next function.
def documented_by_comment():
    pass
