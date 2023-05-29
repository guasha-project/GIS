# DNS
Как происходит разрешение доменов через GIS?

Когда к Гису приходит запрос, он сначала проверяет запрошенный домен по "фильтрам".
Сначала идут фильтры, заданные в опции `hosts` в файле конфигурации.
Они задаются примерно так: `hosts = ["system", "adblock.txt"]`, то есть это файлы hosts, с соответствием IP-адресов и доменов.
Причём, `system` это специальный фильтр, указывающий, что надо подгрузить соответствия из системы.
В Windows это `%SYSTEMROOT%/System32/drivers/etc/hosts`, в Linux это `/etc/hosts`.

Последним фильтром является фильтр блокчейн. Он обращается к базе доменов в блокчейне.
Если там найдена информация по домену, то в ней ищется запрошенная запись, и отдаётся ответ.
Если информация по домену не найдена, но зона такая в блокчейне есть, то отдаётся ответ "не найдено".
Если такой зоны в GIS нет, то он обращается к случайному серверу из опции `forwarders` в файле конфигурации.