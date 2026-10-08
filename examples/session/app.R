library(shiny)
enableBookmarking("server")
ui <- function(request) fluidPage(
  h1("Session compatibility fixture"),
  numericInput("n", "Number", value = 2),
  textOutput("double"), textOutput("identity"), textOutput("pid"),
  actionButton("save", "Save bookmark"), textOutput("bookmark_url")
)
server <- function(input, output, session) {
  output$double <- renderText(input$n * 2)
  output$identity <- renderText(if (is.null(session$user)) "anonymous" else session$user)
  output$pid <- renderText(Sys.getpid())
  saved <- reactiveVal("")
  observeEvent(input$save, session$doBookmark())
  onBookmarked(function(url) saved(url))
  setBookmarkExclude("save")
  output$bookmark_url <- renderText(saved())
}
shinyApp(ui, server)
