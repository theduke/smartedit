package fixtures.scala

object Legacy {
  val status = "draft"
  val compatibility = true

  def message(name: String): String = s"hello, $name"
}
