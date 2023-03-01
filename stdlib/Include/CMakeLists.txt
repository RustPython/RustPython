cmake_minimum_required(VERSION 3.0)
project(LibLocale C)
add_library(liblocale STATIC _localemodule.c)
install(TARGETS liblocale DESTINATION .)