library(shiny)
ui <- fluidPage(
  titlePanel("Hello from ShinyHarbor"),
  tags$p("A real R Shiny app, served by the Rust gateway."),
  sliderInput("n", "Number of observations", min = 10, max = 200, value = 50),
  plotOutput("histogram"),
  textOutput("summary"),
  tags$small("Move the slider to check the live WebSocket connection.")
)
server <- function(input, output, session) {
  output$histogram <- renderPlot(hist(seq_len(input$n), main = "Live Shiny output", xlab = "Observation"))
  output$summary <- renderText(paste("Observations:", input$n))
}
shinyApp(ui, server)
