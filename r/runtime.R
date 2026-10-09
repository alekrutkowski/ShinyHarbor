# Original ShinyHarbor runtime integration, using the installed Shiny package.
harbor_path <- function(path) {
  # Rust canonicalize() uses Windows extended-length paths. R expects ordinary
  # drive/UNC paths; normalizing here also stabilizes bookmark app identifiers.
  path <- sub("^\\\\\\\\\\?\\\\UNC\\\\", "\\\\\\\\", path)
  path <- sub("^\\\\\\\\\\?\\\\", "", path)
  normalizePath(path, winslash = "/", mustWork = TRUE)
}

harbor_bookmarks <- function(app, root) {
  app <- harbor_path(app)
  root <- harbor_path(root)
  account <- Sys.info()[["effective_user"]]
  if (is.null(account) || is.na(account) || !nzchar(account)) account <- Sys.info()[["user"]]
  if (is.null(account) || is.na(account) || !grepl("^[^/\\\\.][^/\\\\]*$", account)) {
    stop("Cannot determine a safe OS account name for bookmark storage")
  }
  # Same directory naming convention as Shiny Server. State is scoped to the
  # application path and OS account, and survives worker/server restarts.
  key <- paste0(basename(app), "-", digest::digest(app, algo = "md5", serialize = FALSE))
  folder <- file.path(root, account, key)
  if (!dir.exists(folder) && !dir.create(folder, recursive = TRUE, mode = "0700")) {
    stop("Cannot create bookmark application directory")
  }
  folder <- harbor_path(folder)
  resolve <- function(id) {
    if (!is.character(id) || length(id) != 1L || is.na(id) ||
        !grepl("^[A-Za-z0-9_-]{1,128}$", id)) stop("Invalid bookmark identifier")
    file.path(folder, id)
  }
  list(
    save = function(id, callback) {
      path <- resolve(id)
      if (file.exists(path) || !dir.create(path, mode = "0700")) stop("Bookmark already exists or cannot be created")
      complete <- FALSE
      on.exit(if (!complete) unlink(path, recursive = TRUE), add = TRUE)
      callback(path)
      complete <- TRUE
      invisible(NULL)
    },
    load = function(id, callback) {
      path <- resolve(id)
      if (!dir.exists(path) || dirname(harbor_path(path)) != folder) stop("Bookmark not found")
      callback(path)
    }
  )
}

harbor_configure <- function(app, bookmarks, secret, sanitize = TRUE) {
  if (!is.character(secret) || length(secret) != 1L || nchar(secret) < 32L) {
    stop("Missing worker authentication secret")
  }
  options(shiny.sanitize.errors = sanitize, shiny.fullstacktrace = FALSE,
          shiny.sharedSecret = secret)
  if (nzchar(Sys.getenv("HARBOR_DISABLE_PROTOCOLS")) || nzchar(Sys.getenv("HARBOR_GOOGLE_ANALYTICS_ID"))) {
    previous_filter <- getOption("shiny.http.response.filter")
    disabled <- jsonlite::fromJSON(Sys.getenv("HARBOR_DISABLE_PROTOCOLS", "[]"))
    disabled <- htmltools::htmlEscape(jsonlite::toJSON(disabled, auto_unbox = FALSE), attribute = TRUE)
    buffer_bytes <- Sys.getenv("HARBOR_SEND_BUFFER_BYTES", "134217728")
    buffer_messages <- Sys.getenv("HARBOR_SEND_BUFFER_MESSAGES", "2048")
    if (!grepl("^[0-9]{1,10}$", buffer_bytes) || !grepl("^[0-9]{1,10}$", buffer_messages)) stop("Invalid transport limits")
    scripts <- paste0('<script src="__assets__/sockjs.min.js"></script>',
      '<script src="__assets__/shinyharbor-client.js" data-worker="',
      htmltools::htmlEscape(Sys.getenv("HARBOR_WORKER_TICKET"), attribute = TRUE),
      '" data-reconnect="', Sys.getenv("HARBOR_RECONNECT", "true"),
      '" data-buffer-bytes="', buffer_bytes, '" data-buffer-messages="', buffer_messages,
      '" data-disabled="', disabled, '"></script>')
    analytics <- Sys.getenv("HARBOR_GOOGLE_ANALYTICS_ID")
    if (nzchar(analytics)) {
      if (!grepl("^[A-Za-z0-9_-]{1,128}$", analytics)) stop("Invalid Google Analytics tag ID")
      scripts <- paste0(scripts,
        '<script async src="https://www.googletagmanager.com/gtag/js?id=', analytics, '"></script>',
        '<script>window.dataLayer=window.dataLayer||[];function gtag(){dataLayer.push(arguments);}',
        'gtag("js",new Date());gtag("config",', jsonlite::toJSON(analytics, auto_unbox = TRUE), ');</script>')
    }
    options(shiny.http.response.filter = function(request, response) {
      if (is.function(previous_filter)) response <- previous_filter(request, response)
      if (response$status == 200L && isTRUE(grepl("^text/html", response$content_type)) &&
          is.character(response$content) && length(response$content) == 1L) {
        response$content <- sub("</head>", paste0(scripts, "</head>"), response$content, fixed = TRUE)
      }
      response
    })
  }
  store <- harbor_bookmarks(app, bookmarks)
  shiny::shinyOptions(save.interface = store$save, load.interface = store$load)
  invisible(store)
}
