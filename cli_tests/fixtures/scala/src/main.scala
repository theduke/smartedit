package fixtures.scala

trait Repository[A] {
  def current(id: String): Option[A]

  def label(value: A): String = s"repo:$value"
}

class Service[T](private val repository: Repository[T]) {
  def run(id: String): String = {
    repository.current(id).map(repository.label).getOrElse("missing")
  }

  object Defaults {
    def label(value: String): String = s"default:$value"

    def create(): Service[String] = new Service[String](new Repository[String] {
      def current(id: String): Option[String] = Some(id)
    })
  }
}

object Service {
  def create(): Service[String] = Defaults.create()
}

def topLevel(value: Int): String = s"value=$value"
