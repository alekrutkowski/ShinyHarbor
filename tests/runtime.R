# Runs against the real installed Shiny package, without starting the Rust server.
source("r/runtime.R")
library(testthat)

test_that("Rust Windows canonical paths normalize for R", {
  skip_if(.Platform$OS.type != "windows")
  normal <- normalizePath(getwd(), winslash = "/", mustWork = TRUE)
  extended <- paste0("\\\\?\\", gsub("/", "\\\\", normal, fixed = TRUE))
  expect_identical(harbor_path(extended), normal)
})

test_that("bookmarks persist across worker integrations and isolate apps", {
  root <- tempfile("harbor-bookmarks-")
  dir.create(root)
  on.exit(unlink(root, recursive = TRUE))
  app <- file.path(root, "app"); dir.create(app)
  other <- file.path(root, "other"); dir.create(other)
  state <- file.path(root, "state"); dir.create(state)
  store <- harbor_bookmarks(app, state)
  store$save("a1b2", function(path) saveRDS(list(n = 7, text = "bookmark"), file.path(path, "values.rds")))
  restarted <- harbor_bookmarks(app, state)
  restored <- restarted$load("a1b2", function(path) readRDS(file.path(path, "values.rds")))
  expect_equal(restored, list(n = 7, text = "bookmark"))
  expect_error(harbor_bookmarks(other, state)$load("a1b2", identity), "not found")
  expect_error(store$save("a1b2", identity), "already exists")
})

test_that("bookmark identifiers cannot traverse or overwrite files", {
  root <- tempfile("harbor-invalid-"); dir.create(root)
  on.exit(unlink(root, recursive = TRUE))
  store <- harbor_bookmarks(root, root)
  for (id in list("../escape", "a/b", "a\\b", "", NA_character_, c("a", "b"), "C:stream", ".", paste(rep("a", 129), collapse=""))) {
    expect_error(store$save(id, identity), "Invalid bookmark")
    expect_error(store$load(id, identity), "Invalid bookmark")
  }
  expect_error(store$save("broken", function(path) stop("callback failed")), "callback failed")
  expect_error(store$load("broken", identity), "not found")
  expect_silent(store$save("broken", identity))
})

test_that("Shiny enforces the per-worker secret and receives bookmark callbacks", {
  root <- tempfile("harbor-config-"); dir.create(root)
  on.exit(unlink(root, recursive = TRUE))
  secret <- paste(rep("s", 48), collapse = "")
  harbor_configure(root, root, secret, sanitize = FALSE)
  check <- shiny:::loadSharedSecret()
  expect_true(check(secret))
  expect_false(check(NULL))
  expect_false(check("forged"))
  expect_false(getOption("shiny.sanitize.errors"))
  expect_type(shiny::getShinyOption("save.interface"), "closure")
  expect_type(shiny::getShinyOption("load.interface"), "closure")
  expect_error(harbor_configure(root, root, "short"), "authentication secret")
})

test_that("the real Shiny session consumes trusted identity credentials", {
  # Use a minimal transport with the same request environment httpuv supplies.
  messages <- list()
  ws <- list(request = new.env(parent = emptyenv()),
    send = function(message) { messages[[length(messages) + 1L]] <<- message },
    close = function(...) NULL)
  ws$request$HTTP_SHINY_SERVER_CREDENTIALS <- '{"user":"alice","groups":[]}'
  session <- shiny:::ShinySession$new(ws)
  expect_identical(session$user, "alice")
  expect_length(session$groups, 0L)
  expect_identical(session$request$HTTP_SHINY_SERVER_CREDENTIALS, ws$request$HTTP_SHINY_SERVER_CREDENTIALS)
  session$close()
})

cat("R runtime checks completed with R ", as.character(getRversion()),
    " and Shiny ", as.character(packageVersion("shiny")), "\n", sep = "")
