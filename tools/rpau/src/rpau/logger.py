import logging
import sys

_NAME = "rpau"


def build_root_logger(
    name: str = _NAME, level: int = logging.WARNING
) -> logging.Logger:
    logger = logging.getLogger(name)
    logger.setLevel(level)
    formatter = logging.Formatter(
        "%(asctime)s - %(name)s - %(levelname)s - %(message)s"
    )

    sh = logging.StreamHandler(sys.stdout)
    sh.setFormatter(formatter)
    logger.handlers.clear()
    logger.addHandler(sh)

    return logger


def get_logger(name: str) -> logging.Logger:
    return logging.getLogger(_NAME).getChild(name)
